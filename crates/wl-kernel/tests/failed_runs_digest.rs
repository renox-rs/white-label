//! Acceptance checks of #458 (BL-KRN-10): the daily digest of failed rule
//! runs to the company admin (`PLT.RUL.03`).
//!
//! The failures are stored with times around the period of the 5 November
//! 2026 digest: 07:00 on the 4th to 07:00 on the 5th, Asia/Dubai (03:00 to
//! 03:00 UTC). The notifier is a recording double.

use std::sync::{Arc, Mutex};

use renox::auth::{Permissions, permissions};
use renox::chrono::NaiveDate;
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_kernel::automation::failed_digest::{self, NOTICE_KIND, run_failed_digest};
use wl_kernel::automation::{FailureKind, RuleRunFailure, RuleRunStatus, run_rule};
use wl_kernel::compliance::{AlertKind, AlertStatus, ComplianceAlert};
use wl_kernel::digest::{self, RecordInfo};
use wl_kernel::notify::{
    MessageId, Notifier, NotifyFuture, NotifyRecipient, NotifyRequest, provide_notifier,
};
use wl_kernel::{ClientConfig, RuleRun, RuleRunSpec, RunOutcome, Subject};

const REMINDER: &str = "FIN.RUL.01";
const CRITICAL: &str = "HRM.RUL.03";

const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

fn at(text: &str) -> DateTime {
    text.parse().expect("an instant")
}

fn nov(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, day).expect("a date")
}

#[derive(Clone, Default)]
struct Notices(Arc<Mutex<Vec<NotifyRequest>>>);

impl Notices {
    fn digests(&self) -> Vec<NotifyRequest> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.type_key == failed_digest::NOTIFICATION_TYPE)
            .cloned()
            .collect()
    }

    fn to(&self, id: i64) -> Vec<NotifyRequest> {
        self.digests()
            .into_iter()
            .filter(|r| r.recipients.contains(&NotifyRecipient::User { id }))
            .collect()
    }
}

impl Notifier for Notices {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(request);
            Ok(Vec::new())
        })
    }
}

struct Billing {
    notices: Notices,
}

impl Module for Billing {
    fn name(&self) -> &'static str {
        "test-billing"
    }

    fn register(&self, app: &mut Registry) {
        provide_notifier(app, self.notices.clone());
        digest::register_info("invoicing.invoice", |_tx, id| {
            Box::pin(async move { Ok(RecordInfo::numbered(&format!("INV-ACME-2026-{id:04}"))) })
        });
        digest::register_rule_title(REMINDER, "invoice reminder");
        digest::register_rule_title(CRITICAL, "WPS file reminder");
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
    notices: Notices,
    _views: tempfile::TempDir,
    admin: User,
    second_admin: User,
    clerk: User,
    ops: User,
}

async fn world() -> World {
    let dir = tempfile::tempdir().expect("a temporary views directory");
    let layout = dir.path().join("layouts/app.html");
    std::fs::create_dir_all(layout.parent().expect("a path")).expect("a directory");
    std::fs::write(layout, LAYOUT).expect("a layout");

    let notices = Notices::default();
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(
        App::new()
            .module(Auth::new())
            .module(Permissions)
            .module(Billing {
                notices: notices.clone(),
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

    let db = app.db();
    permissions::define_role(db, "admin", &[]).await.unwrap();
    permissions::define_role(db, "clerk", &[]).await.unwrap();
    permissions::define_role(db, "ops", &[wl_kernel::permissions::AUTOMATIONS_RETRY])
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
        admin: user("Ada", "admin").await,
        second_admin: user("Abe", "admin").await,
        clerk: user("Carl", "clerk").await,
        ops: user("Olga", "ops").await,
        notices,
        _views: dir,
        app,
    }
}

impl World {
    /// A run of `rule` about invoice `invoice` that ended `status`, and its
    /// failure of `kind` at `when` (UTC).
    async fn failure(
        &self,
        rule: &str,
        invoice: i64,
        key: &str,
        status: RuleRunStatus,
        kind: FailureKind,
        when: &str,
    ) -> RuleRun {
        let db = self.app.db();
        let mut run = RuleRun {
            rule: rule.to_owned(),
            subject_type: "invoicing.invoice".into(),
            subject_id: invoice,
            occurrence_key: key.to_owned(),
            status,
            attempts: 5,
            error: Some("mail server down".into()),
            ..Default::default()
        };
        let mut tx = db.begin().await.unwrap();
        run.insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        RuleRunFailure::create(
            db,
            RuleRunFailure {
                run_id: run.id,
                rule: rule.to_owned(),
                subject_type: "invoicing.invoice".into(),
                subject_id: invoice,
                occurrence_key: key.to_owned(),
                kind,
                error: Some(match kind {
                    FailureKind::Failed => "mail server down".to_owned(),
                    FailureKind::Stopped => "The rule chain is 4 deep; the limit is 3.".to_owned(),
                }),
                at: at(when),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        run
    }

    async fn send(&self, date: NaiveDate) -> failed_digest::Report {
        run_failed_digest(self.app.state(), date)
            .await
            .expect("the digest runs")
    }

    fn text_to(&self, user: &User) -> String {
        let sent = self.notices.to(user.id);
        assert_eq!(sent.len(), 1, "one digest for {}", user.name);
        sent[0].data["digest"].as_str().unwrap().to_owned()
    }
}

/// Done when 1: the ordinary "invoice reminder" failed for
/// INV-ACME-2026-0310 at 2026-11-04 16:20 and nothing else failed: at
/// 2026-11-05 07:00 the admin gets one digest with that line, linking to the
/// run; in the app too. Each company admin gets the same digest.
#[renox::test]
async fn an_ordinary_failure_is_in_the_next_morning_digest_with_a_link_to_the_run() {
    let w = world().await;
    let run = w
        .failure(
            REMINDER,
            310,
            "overdue+7",
            RuleRunStatus::Failed,
            FailureKind::Failed,
            "2026-11-04T12:20:00Z",
        )
        .await;

    let report = w.send(nov(5)).await;
    assert_eq!(report.runs, 1);
    assert_eq!(report.sent, 2, "one message per company admin");
    assert!(w.notices.to(w.clerk.id).is_empty());

    let text = w.text_to(&w.admin);
    assert!(
        text.starts_with(
            "Rule runs that failed or were stopped from 2026-11-04 07:00 to 2026-11-05 07:00"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "- invoice reminder: invoice INV-ACME-2026-0310, occurrence overdue+7; failed at \
             2026-11-04 16:20; last error: mail server down"
        ),
        "{text}"
    );
    assert!(
        text.contains(&format!("/rule-runs/{}", run.id)),
        "the line links to the run: {text}"
    );
    assert_eq!(w.text_to(&w.second_admin), text);

    let bell: Vec<_> = w
        .admin
        .notifications(w.app.db(), 20)
        .await
        .unwrap()
        .into_iter()
        .filter(|n| n.kind == NOTICE_KIND)
        .collect();
    assert_eq!(bell.len(), 1, "in-app as well");
}

/// Done when 2: a run stopped for chain depth at 2026-11-04 20:00 is listed
/// with reason *stopped*.
#[renox::test]
async fn a_stopped_run_is_listed_as_stopped() {
    let w = world().await;
    w.failure(
        REMINDER,
        311,
        "overdue+14",
        RuleRunStatus::Stopped,
        FailureKind::Stopped,
        "2026-11-04T16:00:00Z",
    )
    .await;
    w.send(nov(5)).await;
    let text = w.text_to(&w.admin);
    assert!(
        text.contains("invoice INV-ACME-2026-0311, occurrence overdue+14; stopped at 2026-11-04 20:00"),
        "{text}"
    );
}

/// Done when 3: a compliance-critical run that failed at 2026-11-04 09:00 is
/// listed and marked *alert sent*.
#[renox::test]
async fn a_critical_failure_is_listed_and_marked_alert_sent() {
    let w = world().await;
    w.failure(
        CRITICAL,
        4,
        "wps:2026-11",
        RuleRunStatus::Failed,
        FailureKind::Failed,
        "2026-11-04T05:00:00Z",
    )
    .await;
    ComplianceAlert::create(
        w.app.db(),
        ComplianceAlert {
            rule: CRITICAL.into(),
            subject_type: "invoicing.invoice".into(),
            subject_id: 4,
            occurrence_key: "wps:2026-11".into(),
            kind: AlertKind::Failed,
            owner_role: "admin".into(),
            status: AlertStatus::Open,
            notified_at: Some(at("2026-11-04T05:00:01Z")),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    w.send(nov(5)).await;
    let text = w.text_to(&w.admin);
    assert!(text.contains("WPS file reminder"), "{text}");
    assert!(text.contains("failed at 2026-11-04 09:00"), "{text}");
    assert!(text.contains("(compliance-critical, alert sent)"), "{text}");
}

/// A run rerun with success before 07:00 is still listed, marked *rerun,
/// succeeded*.
#[renox::test]
async fn a_run_rerun_with_success_is_still_listed_and_marked() {
    let w = world().await;
    w.failure(
        REMINDER,
        312,
        "overdue+7",
        RuleRunStatus::Succeeded,
        FailureKind::Failed,
        "2026-11-04T08:00:00Z",
    )
    .await;
    w.send(nov(5)).await;
    let text = w.text_to(&w.admin);
    assert!(text.contains("INV-ACME-2026-0312"), "{text}");
    assert!(text.contains("(rerun, succeeded)"), "{text}");
}

/// Done when 4: a run that failed at 2026-11-04 06:59 is not in the
/// 2026-11-05 digest; it was in the previous one.
#[renox::test]
async fn a_failure_before_seven_belongs_to_the_previous_digest() {
    let w = world().await;
    w.failure(
        REMINDER,
        313,
        "overdue+7",
        RuleRunStatus::Failed,
        FailureKind::Failed,
        "2026-11-04T02:59:00Z",
    )
    .await;
    let report = w.send(nov(5)).await;
    assert_eq!(report.runs, 0);
    assert!(w.notices.digests().is_empty());

    w.send(nov(4)).await;
    assert!(w.text_to(&w.admin).contains("INV-ACME-2026-0313"));
}

/// Done when 5: with no failed or stopped run in the period, no digest is
/// sent.
#[renox::test]
async fn nothing_failed_sends_nothing() {
    let w = world().await;
    let report = w.send(nov(5)).await;
    assert_eq!(report, failed_digest::Report::default());
    assert!(w.notices.digests().is_empty());
}

/// Done when 6: running the job twice on 2026-11-05 sends one digest.
#[renox::test]
async fn running_the_job_twice_sends_one_digest() {
    let w = world().await;
    w.failure(
        REMINDER,
        310,
        "overdue+7",
        RuleRunStatus::Failed,
        FailureKind::Failed,
        "2026-11-04T12:20:00Z",
    )
    .await;
    assert_eq!(w.send(nov(5)).await.sent, 2);
    let again = w.send(nov(5)).await;
    assert_eq!((again.sent, again.skipped), (0, 2));
    assert_eq!(w.notices.to(w.admin.id).len(), 1);
}

/// The `stop` and `fail` transitions record the failure the digest reads,
/// with the run's error, at the time it happened.
#[renox::test]
async fn a_stopped_run_records_its_failure() {
    let w = world().await;
    let before = now();
    let spec = RuleRunSpec::new(REMINDER, Subject::new("invoicing.invoice", 9), "chain")
        .chain_depth(4);
    let outcome = run_rule(w.app.state(), spec, |_state| async { Ok(()) })
        .await
        .unwrap();
    assert_eq!(outcome, RunOutcome::Stopped);
    let failures = RuleRunFailure::query().get(w.app.db()).await.unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].kind, FailureKind::Stopped);
    assert_eq!(failures[0].occurrence_key, "chain");
    assert!(failures[0].error.as_deref().unwrap().contains("4 deep"));
    assert!(failures[0].at >= before);
}

/// The run's page: company admins and holders of `automations.retry` open
/// it; a holder of `automations.retry` runs a failed run again from it.
#[renox::test]
async fn the_run_page_shows_the_run_and_offers_rerun() {
    let w = world().await;
    let run = w
        .failure(
            REMINDER,
            310,
            "overdue+7",
            RuleRunStatus::Failed,
            FailureKind::Failed,
            "2026-11-04T12:20:00Z",
        )
        .await;
    let url = format!("/rule-runs/{}", run.id);

    w.app.acting_as(&w.clerk);
    assert_eq!(w.app.get(&url).await.status.as_u16(), 403);

    w.app.acting_as(&w.admin);
    let page = w.app.get(&url).await;
    page.assert_ok();
    let html = page.text();
    assert!(html.contains("invoice reminder"), "{html}");
    assert!(html.contains("INV-ACME-2026-0310"), "{html}");
    assert!(html.contains("mail server down"), "{html}");

    w.app.acting_as(&w.ops);
    let page = w.app.get(&url).await;
    page.assert_ok();
    assert!(page.text().contains("Run again"));
    let rerun = w.app.post(&format!("{url}/rerun"), &[("x", "y")]).await;
    assert!(rerun.status.is_redirection() || rerun.status.is_success(), "{}", rerun.text());
    let queued = RuleRun::find(w.app.db(), run.id).await.unwrap().unwrap();
    assert_eq!(queued.status, RuleRunStatus::Queued);
}
