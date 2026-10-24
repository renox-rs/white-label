//! Acceptance checks of #32: the owner alert when a compliance-critical rule
//! fails, is stopped or misses its schedule.
//!
//! The rules are registered by a test-only module: `HRM.RUL.03` (owner role
//! *md*), `DOC.RUL.03` (nightly 01:00) and `DOC.RUL.04` (nightly 01:10), the
//! last two owned by the deployment's business owner role (`[roles] owner =
//! "md"`). `TST.RUL.99` is not critical. Notices and tasks are recorded by
//! test doubles.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use renox::auth::{Permissions, permissions};
use renox::chrono::Duration as Span;
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_kernel::automation::{RuleRunStatus, run_rule, transitions};
use wl_kernel::compliance::{
    self, AlertKind, AlertStatus, ClosedHow, ComplianceAlert, occurrence_key, planned_daily,
    schedule_subject,
};
use wl_kernel::notify::{
    MessageId, Notifier, NotifyChannel, NotifyFuture, NotifyRecipient, NotifyRequest, Urgency,
    provide_notifier,
};
use wl_kernel::tickets::{
    Assignee, OpenTicket, TaskCreator, TicketFuture, TicketId, TicketRef, provide_task_creator,
};
use wl_kernel::{
    AutomationFailed, ClientConfig, ComplianceRule, CriticalRule, RuleRun, RuleRunSpec, RunOutcome,
    Subject,
};

const WPS: &str = "HRM.RUL.03";
const NIGHTLY: &str = "DOC.RUL.03";
const LATER: &str = "DOC.RUL.04";
const PLAIN: &str = "TST.RUL.99";

/// The pages extend the app's layout; a test has no app, so it brings the
/// smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

#[derive(Clone, Default)]
struct Doubles(Arc<Mutex<Recorded>>);

#[derive(Default)]
struct Recorded {
    notices: Vec<NotifyRequest>,
    opened: Vec<OpenTicket>,
    completed: Vec<(i64, String)>,
    /// While set, the notifier fails.
    notifier_down: bool,
}

impl Doubles {
    fn notices(&self) -> Vec<NotifyRequest> {
        self.0.lock().unwrap().notices.clone()
    }

    fn alerts_sent(&self) -> usize {
        self.notices()
            .iter()
            .filter(|n| n.type_key == compliance::service::NOTIFICATION_TYPE)
            .count()
    }

    /// The user ids the alerts went to, sorted.
    fn alerted_users(&self) -> Vec<i64> {
        let mut ids: Vec<i64> = self
            .notices()
            .iter()
            .filter(|n| n.type_key == compliance::service::NOTIFICATION_TYPE)
            .flat_map(|n| n.recipients.clone())
            .filter_map(|r| match r {
                NotifyRecipient::User { id } => Some(id),
                _ => None,
            })
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    fn set_notifier_down(&self, down: bool) {
        self.0.lock().unwrap().notifier_down = down;
    }

    fn opened(&self) -> Vec<OpenTicket> {
        self.0.lock().unwrap().opened.clone()
    }

    fn completed(&self) -> Vec<(i64, String)> {
        self.0.lock().unwrap().completed.clone()
    }
}

impl Notifier for Doubles {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        Box::pin(async move {
            let mut recorded = self.0.lock().unwrap();
            if recorded.notifier_down {
                return Err(Error::Internal(renox::anyhow::anyhow!(
                    "the notifier is down"
                )));
            }
            recorded.notices.push(request);
            Ok(Vec::new())
        })
    }
}

impl TaskCreator for Doubles {
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

    fn complete<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        note: &'a str,
    ) -> TicketFuture<'a, bool> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap()
                .completed
                .push((ticket.0, note.to_owned()));
            Ok(true)
        })
    }
}

struct Hr {
    doubles: Doubles,
}

impl Module for Hr {
    fn name(&self) -> &'static str {
        "test-hr"
    }

    fn register(&self, app: &mut Registry) {
        provide_notifier(app, self.doubles.clone());
        provide_task_creator(app, self.doubles.clone());
        CriticalRule::new(WPS).owner_role("md").register();
        CriticalRule::new(NIGHTLY).daily_at("01:00").register();
        CriticalRule::new(LATER).daily_at("01:10").register();
    }

    fn routes(&self) -> Routes {
        Routes::new().post("/test/rule-runs/{id}/rerun", rerun)
    }
}

/// The `rerun` transition (`automations.retry`) as the logged-in user: the
/// machine checks the permission.
async fn rerun(
    State(state): State<AppState>,
    user: renox::AuthUser,
    Path(id): Path<i64>,
) -> Result<String> {
    let run = RuleRun::find_or_404(&state.db, id).await?;
    let _done = run
        .transition(transitions::RERUN)
        .by(&user)
        .run(&state)
        .await
        .map_err(renox::state_machine::TransitionError::into_error)?;
    Ok("queued".to_owned())
}

const CLIENT: &str = r#"
[client]
name = "Acme Manpower LLC"
app = "manpower"

[modules]
enabled = []

[roles]
owner = "md"

[[legal_entities]]
code = "ACME"
name_en = "Acme Manpower LLC"
default = true
"#;

struct World {
    app: TestApp,
    doubles: Doubles,
    _views: tempfile::TempDir,
    admin: User,
    md: User,
    clerk: User,
}

async fn world() -> World {
    let dir = tempfile::tempdir().expect("a temporary views directory");
    let layout = dir.path().join("layouts/app.html");
    std::fs::create_dir_all(layout.parent().expect("a path")).expect("a directory");
    std::fs::write(layout, LAYOUT).expect("a layout");

    let doubles = Doubles::default();
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(
        App::new()
            .module(Auth::new())
            .module(Permissions)
            .module(Hr {
                doubles: doubles.clone(),
            }),
        config,
    )
    .expect("the kernel applies");
    let views = dir.path().to_path_buf();
    let app = TestApp::with_config(app, move |c| c.views_path = views.clone()).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    app.fake_events();

    let db = app.db();
    permissions::define_role(db, "admin", &[]).await.unwrap();
    permissions::define_role(db, "md", &[]).await.unwrap();
    permissions::define_role(db, "clerk", &[]).await.unwrap();
    let user = |name: &'static str, role: &'static str| async move {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(db, name, &email, "password123")
            .await
            .unwrap();
        user.assign_role(db, role).await.unwrap();
        user
    };
    World {
        admin: user("Ada", "admin").await,
        md: user("Maya", "md").await,
        clerk: user("Carl", "clerk").await,
        doubles,
        _views: dir,
        app,
    }
}

impl World {
    /// Runs `rule` about payroll run 4 for `key`, depth `depth`, with an
    /// action that fails when `fail`, at the travelled time.
    async fn run(&self, rule: &str, key: &str, depth: i64, fail: bool) -> RunOutcome {
        let spec =
            RuleRunSpec::new(rule, Subject::new("hr.payroll_run", 4), key).chain_depth(depth);
        self.app
            .at_travelled_time(run_rule(self.app.state(), spec, move |_state| async move {
                if fail {
                    Err(Error::Status(
                        StatusCode::BAD_GATEWAY,
                        "bank file rejected".to_owned(),
                    ))
                } else {
                    Ok(())
                }
            }))
            .await
            .expect("the rule runs")
    }

    /// Five failing attempts, with the backoff between them.
    async fn fail_for_good(&self, rule: &str, key: &str) {
        for wait in [60_u64, 300, 900, 3600] {
            assert_eq!(self.run(rule, key, 0, true).await, RunOutcome::Retrying);
            self.app.travel(Duration::from_secs(wait));
        }
        assert_eq!(self.run(rule, key, 0, true).await, RunOutcome::Failed);
    }

    async fn alerts(&self) -> Vec<ComplianceAlert> {
        ComplianceAlert::query()
            .order_by(ComplianceAlert::ID)
            .get(self.app.db())
            .await
            .unwrap()
    }

    /// Moves the clock to `at` (which must not be in the past), rounded up to
    /// the next whole second so the clock is never just before `at`.
    fn travel_to(&self, at: DateTime) {
        let seconds = ((at - now()).num_milliseconds().max(0) + 999) / 1000;
        self.app
            .travel(Duration::from_secs(u64::try_from(seconds).unwrap_or(0)));
    }

    /// The next 02:00 Asia/Dubai at least three hours from now.
    fn two_am(&self) -> DateTime {
        let from = now() + Span::hours(3);
        planned_daily("TST.RUL.00", "02:00", from, from + Span::hours(30))[0].planned_at
    }

    /// Starts watching the nightly rules three hours before `at`.
    async fn watch_since(&self, at: DateTime) {
        for rule in [NIGHTLY, LATER] {
            ComplianceRule::where_eq("rule", rule)
                .update(
                    self.app.db(),
                    &[("watched_since", &Some(at - Span::hours(3)))],
                )
                .await
                .unwrap();
        }
    }

    /// `user`'s in-app owner alerts, as JSON text.
    async fn bell(&self, user: &User) -> Vec<String> {
        user.notifications(self.app.db(), 50)
            .await
            .unwrap()
            .into_iter()
            .filter(|n| n.kind == compliance::service::ALERT_KIND)
            .map(|n| n.data.to_string())
            .collect()
    }

    /// A user who holds `automations.retry`.
    async fn operator(&self) -> User {
        permissions::define_role(
            self.app.db(),
            "ops",
            &[wl_kernel::permissions::AUTOMATIONS_RETRY],
        )
        .await
        .unwrap();
        let user = User::register(self.app.db(), "Olga", "olga@example.com", "password123")
            .await
            .unwrap();
        user.assign_role(self.app.db(), "ops").await.unwrap();
        user
    }

    /// Runs the check of the scheduled task at the travelled time.
    async fn check(&self) {
        self.app
            .run_scheduled(compliance::service::TASK)
            .await
            .expect("the check runs");
    }
}

/// AC1: a critical rule's run that fails after 5 attempts alerts the MD
/// role and the company admin at once, gives the MD role a task and emits
/// `AutomationFailed { compliance_critical: true }`.
#[renox::test]
async fn a_failed_critical_run_alerts_the_owner_and_the_admin_and_opens_a_task() {
    let w = world().await;
    w.fail_for_good(WPS, "wps:2026-11").await;

    let alerts = w.alerts().await;
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].kind, AlertKind::Failed);
    assert_eq!(alerts[0].owner_role, "md");
    assert_eq!(alerts[0].error.as_deref(), Some("bank file rejected"));
    assert_eq!(alerts[0].status, AlertStatus::Open);

    let mut expected = vec![w.admin.id, w.md.id];
    expected.sort_unstable();
    assert_eq!(w.doubles.alerted_users(), expected);
    assert_eq!(w.doubles.alerts_sent(), 1);
    // The e-mail side: one request through the notice contract, whose type
    // has an e-mail template and is urgent.
    let ty = compliance::service::notification_type();
    assert_eq!(ty.urgency, Urgency::Urgent);
    assert!(
        ty.templates
            .iter()
            .any(|t| t.channel == NotifyChannel::Email)
    );
    // The in-app side: a notice in the bell of each of the two users, none
    // for the clerk.
    for user in [&w.md, &w.admin] {
        let bell = w.bell(user).await;
        assert_eq!(bell.len(), 1, "{}", user.name);
        assert!(bell[0].contains(WPS), "{}", bell[0]);
        assert!(bell[0].contains("failed"), "{}", bell[0]);
    }
    assert!(w.bell(&w.clerk).await.is_empty());
    let notice = &w.doubles.notices()[0];
    assert_eq!(notice.data["rule"], WPS);
    assert!(notice.data["what"].as_str().unwrap().contains("failed"));

    let tasks = w.doubles.opened();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee, Some(Assignee::user(w.md.id)));
    assert!(tasks[0].title.contains(WPS), "{}", tasks[0].title);
    assert_eq!(alerts[0].ticket_number.as_deref(), Some("TKT-2026-0001"));

    w.app
        .assert_emitted::<AutomationFailed>(|e| e.rule == WPS && e.compliance_critical);
}

/// A run that is only retrying alerts nobody.
#[renox::test]
async fn a_retrying_run_does_not_alert() {
    let w = world().await;
    assert_eq!(
        w.run(WPS, "wps:2026-11", 0, true).await,
        RunOutcome::Retrying
    );
    assert!(w.alerts().await.is_empty());
    assert_eq!(w.doubles.alerts_sent(), 0);
    assert!(w.doubles.opened().is_empty());
}

/// AC2: a stopped run (chain depth above 3) of a critical rule does the same.
#[renox::test]
async fn a_stopped_critical_run_does_the_same() {
    let w = world().await;
    assert_eq!(w.run(WPS, "deep", 4, false).await, RunOutcome::Stopped);

    let alerts = w.alerts().await;
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].kind, AlertKind::Stopped);
    assert_eq!(w.doubles.alerts_sent(), 1);
    assert_eq!(w.doubles.opened().len(), 1);
    w.app
        .assert_emitted::<AutomationFailed>(|e| e.compliance_critical);
}

/// AC3: a nightly rule at 01:00 that has not run: the check at 02:00 finds it
/// missed and creates the alert and task; the checks at 02:15 and 02:30
/// create nothing more.
#[renox::test]
async fn a_missed_nightly_run_is_found_at_the_check_once() {
    let w = world().await;
    let two = w.two_am();
    w.watch_since(two).await;

    w.travel_to(two);
    w.check().await;
    let alerts = w.alerts().await;
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0].rule, NIGHTLY);
    assert_eq!(alerts[0].kind, AlertKind::Missed);
    assert_eq!(alerts[0].planned_at, Some(two - Span::hours(1)));
    assert_eq!(alerts[0].owner_role, "md", "the deployment's owner role");
    assert_eq!(w.doubles.alerts_sent(), 1);
    assert_eq!(w.doubles.opened().len(), 1);

    for _ in 0..2 {
        w.app.travel(Duration::from_secs(15 * 60));
        w.check().await;
    }
    // The one planned at 01:10 was found at 02:15 (65 minutes); nothing
    // else was added at 02:30.
    let rules: Vec<_> = w.alerts().await.iter().map(|a| a.rule.clone()).collect();
    assert_eq!(rules, [NIGHTLY, LATER]);
    assert_eq!(w.doubles.alerts_sent(), 2);
}

/// AC4: the same rule planned at 01:10 with no run is not found by the check
/// at 02:00 (50 minutes) and is found at 02:15 (65 minutes).
#[renox::test]
async fn fifty_minutes_late_is_not_missed_and_sixty_five_is() {
    let w = world().await;
    let two = w.two_am();
    w.watch_since(two).await;

    w.travel_to(two);
    w.check().await;
    assert!(w.alerts().await.iter().all(|a| a.rule != LATER));

    w.app.travel(Duration::from_secs(15 * 60));
    w.check().await;
    let found = w.alerts().await;
    let later = found
        .iter()
        .find(|a| a.rule == LATER)
        .expect("found at 02:15");
    assert_eq!(later.planned_at, Some(two - Span::minutes(50)));
}

/// An occurrence that has a run, in whatever state, is not missed; so is one
/// planned before the rule was watched.
#[renox::test]
async fn an_occurrence_with_a_run_or_before_the_watch_is_not_missed() {
    let w = world().await;
    let two = w.two_am();
    w.watch_since(two).await;

    // A run that is still retrying counts as a run.
    let key = occurrence_key(NIGHTLY, wl_kernel::documents::dubai_date(two));
    let spec = RuleRunSpec::new(NIGHTLY, schedule_subject(), key);
    w.app
        .at_travelled_time(run_rule(w.app.state(), spec, |_state| async {
            Err(Error::Status(StatusCode::BAD_GATEWAY, "later".to_owned()))
        }))
        .await
        .unwrap();
    // Watching only starts at 23:00 the evening before: nothing earlier counts.
    ComplianceRule::where_eq("rule", LATER)
        .update(
            w.app.db(),
            &[("watched_since", &Some(two - Span::minutes(30)))],
        )
        .await
        .unwrap();

    w.travel_to(two + Span::hours(2));
    w.check().await;
    assert!(w.alerts().await.is_empty(), "{:?}", w.alerts().await);
}

/// AC5: when the failed run is rerun successfully, the task closes.
#[renox::test]
async fn a_successful_rerun_closes_the_alert_and_the_task() {
    let w = world().await;
    w.fail_for_good(WPS, "wps:2026-11").await;
    let run = RuleRun::query()
        .where_(RuleRun::OCCURRENCE_KEY.eq("wps:2026-11"))
        .first(w.app.db())
        .await
        .unwrap()
        .expect("the run");
    assert_eq!(run.status, RuleRunStatus::Failed);

    // The `rerun` transition (automations.retry) through a request: a clerk
    // without the permission is refused, an operator who holds it queues the
    // run again.
    let url = format!("/test/rule-runs/{}/rerun", run.id);
    w.app.acting_as(&w.clerk);
    let denied = w.app.post(&url, &[("x", "y")]).await;
    assert_eq!(denied.status.as_u16(), 403, "{}", denied.text());
    assert_eq!(
        RuleRun::find(w.app.db(), run.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RuleRunStatus::Failed
    );
    let ops = w.operator().await;
    w.app.acting_as(&ops);
    w.app.post(&url, &[("x", "y")]).await.assert_ok();
    let queued = RuleRun::find(w.app.db(), run.id).await.unwrap().unwrap();
    assert_eq!(queued.status, RuleRunStatus::Queued);
    assert_eq!(queued.attempts, 0);
    assert_eq!(w.alerts().await[0].status, AlertStatus::Open);
    assert_eq!(
        w.run(WPS, "wps:2026-11", 0, false).await,
        RunOutcome::Succeeded
    );

    let alert = w.alerts().await.remove(0);
    assert_eq!(alert.status, AlertStatus::Closed);
    assert_eq!(alert.closed_how, Some(ClosedHow::Rerun));
    assert!(
        alert
            .note
            .as_deref()
            .unwrap()
            .starts_with("Rule ran successfully at ")
    );
    let done = w.doubles.completed();
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].0, 1);
    assert!(done[0].1.starts_with("Rule ran successfully at "));
}

/// The run a missed occurrence was waiting for closes its alert too.
#[renox::test]
async fn the_late_run_of_a_missed_occurrence_closes_its_alert() {
    let w = world().await;
    let two = w.two_am();
    w.watch_since(two).await;
    w.travel_to(two);
    w.check().await;
    assert_eq!(w.alerts().await.len(), 1);

    let key = occurrence_key(NIGHTLY, wl_kernel::documents::dubai_date(two));
    let spec = RuleRunSpec::new(NIGHTLY, schedule_subject(), key);
    let outcome = w
        .app
        .at_travelled_time(run_rule(w.app.state(), spec, |_state| async { Ok(()) }))
        .await
        .unwrap();
    assert_eq!(outcome, RunOutcome::Succeeded);
    assert_eq!(w.alerts().await[0].status, AlertStatus::Closed);
    assert_eq!(w.doubles.completed().len(), 1);
}

/// AC6: closing the task by hand without a note is refused; with a note it
/// closes, with who and what; someone without the owner role is refused.
#[renox::test]
async fn the_owner_closes_the_alert_with_a_note() {
    let w = world().await;
    w.fail_for_good(WPS, "wps:2026-11").await;
    let id = w.alerts().await[0].id;
    let url = format!("/compliance-alerts/{id}/close");

    w.app.acting_as(&w.md);
    let blank = w.app.htmx().post(&url, &[("note", "  ")]).await;
    assert_eq!(blank.status.as_u16(), 422, "{}", blank.text());
    blank.assert_invalid("note");
    assert_eq!(w.alerts().await[0].status, AlertStatus::Open);

    w.app.acting_as(&w.clerk);
    let denied = w.app.post(&url, &[("note", "Paid by hand")]).await;
    assert_eq!(denied.status.as_u16(), 403, "{}", denied.text());
    assert_eq!(w.alerts().await[0].status, AlertStatus::Open);

    w.app.acting_as(&w.md);
    let ok = w
        .app
        .post(
            &url,
            &[("note", "Paid by hand on 4 Nov"), ("attachment", "12")],
        )
        .await;
    assert_eq!(ok.status.as_u16(), 303, "{}", ok.text());
    let alert = w.alerts().await.remove(0);
    assert_eq!(alert.status, AlertStatus::Closed);
    assert_eq!(alert.closed_how, Some(ClosedHow::Manual));
    assert_eq!(alert.closed_by, Some(w.md.id));
    assert_eq!(alert.note.as_deref(), Some("Paid by hand on 4 Nov"));
    assert_eq!(alert.attachment_document_id, Some(12));
    assert_eq!(
        w.doubles.completed(),
        [(1, "Paid by hand on 4 Nov".to_owned())]
    );

    // It is closed once.
    let again = w.app.post(&url, &[("note", "again")]).await;
    assert_eq!(again.status.as_u16(), 422, "{}", again.text());
}

/// A company admin may record it too; a bad attachment number is refused.
#[renox::test]
async fn the_admin_may_close_it_and_the_attachment_is_a_number() {
    let w = world().await;
    w.fail_for_good(WPS, "wps:2026-11").await;
    let id = w.alerts().await[0].id;
    let url = format!("/compliance-alerts/{id}/close");

    w.app.acting_as(&w.admin);
    let bad = w
        .app
        .htmx()
        .post(&url, &[("note", "Paid"), ("attachment", "proof.pdf")])
        .await;
    assert_eq!(bad.status.as_u16(), 422, "{}", bad.text());
    bad.assert_invalid("attachment");
    let ok = w.app.post(&url, &[("note", "Paid by hand")]).await;
    assert_eq!(ok.status.as_u16(), 303, "{}", ok.text());
    assert_eq!(w.alerts().await[0].closed_by, Some(w.admin.id));
}

/// AC7: a failed run of a rule that is not compliance-critical produces no
/// alert, no in-app notice and no task. (The digest line of the failed run
/// belongs to the digest `PLT.RUL.03`, which no module provides yet; see
/// DEFERRED-G1.)
#[renox::test]
async fn a_failed_run_of_an_ordinary_rule_alerts_nobody() {
    let w = world().await;
    w.fail_for_good(PLAIN, "plain").await;

    assert!(w.alerts().await.is_empty());
    assert_eq!(w.doubles.alerts_sent(), 0);
    assert!(w.doubles.opened().is_empty());
    for user in [&w.md, &w.admin, &w.clerk] {
        assert!(w.bell(user).await.is_empty());
    }
    w.app
        .assert_emitted::<AutomationFailed>(|e| e.rule == PLAIN && !e.compliance_critical);
}

/// One alert and one task per occurrence, not per call.
#[renox::test]
async fn an_occurrence_is_raised_once() {
    let w = world().await;
    let raised = || {
        compliance::Raised::new(
            WPS,
            Subject::new("hr.payroll_run", 4),
            "k",
            AlertKind::Failed,
        )
    };
    let first = compliance::raise(w.app.state(), raised()).await.unwrap();
    assert!(first.is_some());
    assert!(
        compliance::raise(w.app.state(), raised())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(w.alerts().await.len(), 1);
    assert_eq!(w.doubles.alerts_sent(), 1);
    assert_eq!(w.doubles.opened().len(), 1);
}

/// Regression: the alert row is stored before the notice. A notifier that
/// fails then must not leave the occurrence "raised" with nobody told and no
/// task: the next raise for the same occurrence finishes the job, once.
#[renox::test]
async fn a_raise_whose_notice_failed_is_finished_by_the_next_raise() {
    let w = world().await;
    let raised = || {
        compliance::Raised::new(
            WPS,
            Subject::new("hr.payroll_run", 4),
            "k",
            AlertKind::Failed,
        )
    };
    w.doubles.set_notifier_down(true);
    assert!(compliance::raise(w.app.state(), raised()).await.is_err());
    assert_eq!(w.doubles.alerts_sent(), 0);
    assert!(w.doubles.opened().is_empty());

    w.doubles.set_notifier_down(false);
    let alert = compliance::raise(w.app.state(), raised())
        .await
        .unwrap()
        .expect("the retry finishes the alert");
    assert!(alert.ticket_number.is_some(), "the task is opened");
    assert_eq!(w.alerts().await.len(), 1, "still one alert");
    assert_eq!(w.doubles.alerted_users().len(), 2, "the MD and the admin");
    assert_eq!(w.doubles.opened().len(), 1);

    assert!(
        compliance::raise(w.app.state(), raised())
            .await
            .unwrap()
            .is_none(),
        "a third call does nothing"
    );
    assert_eq!(w.doubles.alerts_sent(), 1);
    assert_eq!(w.doubles.opened().len(), 1);
}

/// The page lists the open alerts the viewer may act on.
#[renox::test]
async fn the_page_shows_the_alerts_of_the_viewers_roles() {
    let w = world().await;
    w.fail_for_good(WPS, "wps:2026-11").await;

    w.app.acting_as(&w.md);
    let page = w.app.get("/compliance-alerts").await;
    page.assert_ok();
    assert!(page.text().contains(WPS), "{}", page.text());
    // #530: the record reads in words, never as its type and number.
    assert!(page.text().contains("Payroll run #4"), "{}", page.text());
    assert!(!page.text().contains("hr.payroll_run:4"), "{}", page.text());
    w.app.acting_as(&w.admin);
    assert!(w.app.get("/compliance-alerts").await.text().contains(WPS));
    w.app.acting_as(&w.clerk);
    let none = w.app.get("/compliance-alerts").await;
    none.assert_ok();
    assert!(!none.text().contains(WPS), "{}", none.text());
}

#[renox::test]
async fn the_flag_is_stored_and_the_templates_compile() {
    let w = world().await;
    let row = ComplianceRule::of(w.app.db(), WPS).await.unwrap().unwrap();
    assert!(row.critical);
    w.app.assert_views_compile();
}
