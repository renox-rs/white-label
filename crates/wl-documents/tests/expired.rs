//! Acceptance checks of emitting the blocked records when a blocking
//! document expires and lifting the block on renewal (#49, AC1-AC7): the
//! nightly `expiry::run` (`DOC.RUL.03`), the recording notifier and task
//! creator and the deployments of `support`. Dates are relative to today:
//! the permit expires on `day(30)`, so `day(31)` is the first night after.

mod support;

use renox::chrono::NaiveDate;
use wl_documents::expiry::{self, ExpiredLog, RenewalTask, expired, renewal};
use wl_documents::{Document, DocumentStatus};
use wl_kernel::documents::{self, DocumentExpired};
use wl_kernel::tickets::{Assignee, TicketId};
use wl_kernel::{AutomationFailed, BlockMode, COMPANY_ADMIN_ROLE, Subject};

use support::{Booted, OWNER, Options, RAVI, boot, boot_options, day, permit, ravi, site_card};

const ACTIVATE: &str = "deployment.activate";
const RESUME: &str = "deployment.resume";

/// The app with the permit kind, the staff holding the roles and Ravi's
/// deployments: 70 is running, 71 is planned for `day(40)`.
struct World {
    t: Booted,
    pia: i64,
    hal: i64,
    mo: i64,
    rita: renox::prelude::User,
}

async fn world_with(t: Booted, kinds: &[wl_documents::KindSeed]) -> World {
    t.install(kinds).await;
    t.roles(&["pro", "hr", OWNER, COMPANY_ADMIN_ROLE]).await;
    let pia = t.staff("Pia", "pro", None).await.id;
    let hal = t.staff("Hal", "hr", None).await.id;
    let mo = t.staff("Mo", OWNER, None).await.id;
    let rita = t.reviewer().await;
    t.plan.active_deployment(RAVI, 70, day(-10));
    t.plan.planned_deployment(RAVI, 71, day(40));
    World {
        t,
        pia,
        hal,
        mo,
        rita,
    }
}

async fn owner_options() -> Booted {
    boot_options(Options {
        owner: Some(OWNER),
        ..Options::default()
    })
    .await
}

async fn world() -> World {
    world_with(owner_options().await, &[permit()]).await
}

/// Ravi's work permit, expiring on `day(30)`.
async fn ravi_permit(w: &World) -> Document {
    w.t.accepted(&w.rita, ravi(), "work_permit", Some(day(30)))
        .await
}

async fn open_task(t: &Booted, kind: &str) -> Option<RenewalTask> {
    let kind = t.kind(kind).await;
    RenewalTask::find_open(t.db(), "hr.employee", RAVI, kind.id)
        .await
        .unwrap()
}

/// Whether the guard allows resuming Ravi's deployment on `on_date`.
async fn may_resume(t: &Booted, on_date: NaiveDate) -> bool {
    documents::guard(t.state())
        .check(t.state(), RESUME, RAVI, on_date)
        .await
        .expect("the check runs")
        .is_ok()
}

fn records(group: &[wl_kernel::documents::PlannedRecord]) -> Vec<(String, i64)> {
    group
        .iter()
        .map(|r| (r.record.subject_type.clone(), r.record.subject_id))
        .collect()
}

/// AC1: the expired work permit is emitted once with its `blocks` and the
/// running and planned deployments; PRO, HR and the MD are told once each
/// and the renewal task goes to the PRO.
#[renox::test]
async fn an_expired_permit_emits_its_blocked_records_and_opens_the_task() {
    let w = world().await;
    let t = &w.t;
    let document = ravi_permit(&w).await;

    let report = expiry::run(t.state(), day(31)).await.unwrap();

    assert_eq!(
        (
            report.expired,
            report.expired_events,
            report.messages,
            report.tasks,
            report.failed
        ),
        (1, 1, 3, 1, 0),
        "{report:?}"
    );
    let events = t.app.emitted::<DocumentExpired>();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.document, document.document_id());
    assert_eq!(event.subject, ravi());
    assert_eq!(event.kind, "work_permit");
    assert_eq!(event.expires_on, day(30));
    let mut guarded: Vec<&str> = event.blocks.iter().map(|b| b.guarded.as_str()).collect();
    guarded.sort_unstable();
    assert_eq!(guarded, [ACTIVATE, RESUME]);
    assert!(
        event
            .blocks
            .iter()
            .all(|b| b.kind == "work_permit" && b.mode == BlockMode::Block && b.locked)
    );
    assert_eq!(event.blocked.len(), 2);
    for group in &event.blocked {
        assert!([ACTIVATE, RESUME].contains(&group.guarded.as_str()));
        assert_eq!(group.mode, BlockMode::Block);
        assert_eq!(records(&group.open), [("ops.deployment".to_owned(), 70)]);
        assert_eq!(records(&group.planned), [("ops.deployment".to_owned(), 71)]);
        assert_eq!(group.planned[0].date, day(40));
    }

    assert_eq!(t.notices.sent().len(), 3);
    for user in [w.pia, w.hal, w.mo] {
        let sent = t.notices.to_user(user);
        assert_eq!(sent.len(), 1, "user {user}");
        assert_eq!(sent[0].type_key, expired::NOTIFICATION_TYPE);
        assert_eq!(sent[0].type_key, "documents.expired");
        assert_eq!(sent[0].data["subject_label"], "Ravi S.");
        assert_eq!(sent[0].data["kind_name"], "Work permit");
    }
    let owner = t.notices.to_user(w.mo);
    assert_eq!(owner[0].data["items"].as_array().unwrap().len(), 4);

    let opened = t.tasks.opened();
    assert_eq!(opened.len(), 1);
    assert_eq!(
        opened[0].source_key.as_deref(),
        Some("documents.renewal:hr.employee:7:work_permit")
    );
    assert_eq!(opened[0].title, "Renew Work permit of Ravi S.");
    assert_eq!(opened[0].category_key, "task");
    assert_eq!(opened[0].assignee, Some(Assignee::user(w.pia)));
    assert_eq!(opened[0].due_at, Some(renewal::due_at(day(31))));
    let row = open_task(t, "work_permit").await.expect("the task row");
    assert_eq!(row.expired_document_id, document.id);
    assert_eq!(row.ticket_id, Some(1));
    assert_eq!(row.ticket_number.as_deref(), Some("TKT-2026-0001"));
    assert!(ExpiredLog::is_logged(t.db(), document.id).await.unwrap());
}

/// AC2: the expiry date itself is silent; the first night after emits; the
/// same night again and the nights after add nothing.
#[renox::test]
async fn the_event_is_emitted_once() {
    let w = world().await;
    let t = &w.t;
    ravi_permit(&w).await;

    let on_the_day = expiry::run(t.state(), day(30)).await.unwrap();
    assert_eq!(
        (on_the_day.expired, on_the_day.expired_events),
        (0, 0),
        "{on_the_day:?}"
    );
    // The look-ahead of #48 may still tell about the planned work that day;
    // the expiry rule says nothing.
    assert!(
        t.notices
            .sent()
            .iter()
            .all(|n| n.type_key != expired::NOTIFICATION_TYPE)
    );
    assert!(t.tasks.opened().is_empty());
    assert!(t.app.emitted::<DocumentExpired>().is_empty());
    t.notices.clear();

    expiry::run(t.state(), day(31)).await.unwrap();
    assert_eq!(t.app.emitted::<DocumentExpired>().len(), 1);
    t.notices.clear();

    for night in [31, 32] {
        let again = expiry::run(t.state(), day(night)).await.unwrap();
        assert_eq!(
            (
                again.expired_events,
                again.messages,
                again.tasks,
                again.failed
            ),
            (0, 0, 0, 0),
            "night {night}: {again:?}"
        );
    }
    assert!(t.notices.sent().is_empty());
    assert_eq!(t.tasks.opened().len(), 1);
    assert_eq!(t.app.emitted::<DocumentExpired>().len(), 1);
}

/// AC3: the renewed permit is accepted; `DocumentAccepted` is emitted, the
/// old permit is superseded, the renewal task closes with the renewal
/// document, `complete` is called once and resuming is allowed again.
#[renox::test]
async fn an_accepted_renewal_closes_the_task_and_lifts_the_block() {
    let w = world_with(
        boot_options(Options {
            owner: Some(OWNER),
            fake_events: false,
            ..Options::default()
        })
        .await,
        &[permit()],
    )
    .await;
    let t = &w.t;
    let old = ravi_permit(&w).await;
    expiry::run(t.state(), day(31)).await.unwrap();
    assert!(!may_resume(t, day(31)).await, "refused while expired");
    assert!(open_task(t, "work_permit").await.is_some());
    assert!(t.tasks.completed().is_empty());
    let seen = t.accepted_events().len();

    let renewal = t
        .accepted(&w.rita, ravi(), "work_permit", Some(day(365)))
        .await;

    assert_eq!(renewal.status, DocumentStatus::Accepted);
    let events = t.accepted_events();
    assert_eq!(events.len(), seen + 1);
    let accepted = events.last().unwrap();
    assert_eq!(accepted.document, renewal.document_id());
    assert_eq!(accepted.subject, ravi());
    assert_eq!(accepted.kind, "work_permit");
    assert_eq!(accepted.expires_on, Some(day(365)));
    assert_eq!(t.reload(&old).await.status, DocumentStatus::Superseded);

    assert!(open_task(t, "work_permit").await.is_none());
    let kind = t.kind("work_permit").await;
    let rows = RenewalTask::for_subject(t.db(), "hr.employee", RAVI, kind.id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].closed_at.is_some());
    assert_eq!(rows[0].renewal_document_id, Some(renewal.id));
    assert_eq!(
        t.tasks.completed(),
        [(
            TicketId(1),
            format!(
                "Renewed: Work permit, valid until {}",
                day(365).format("%-d %b %Y")
            )
        )]
    );
    assert!(may_resume(t, day(31)).await, "allowed again");
    assert!(may_resume(t, day(0)).await);
    // Nothing is resumed or flagged here, and the next night is quiet.
    let quiet = expiry::run(t.state(), day(32)).await.unwrap();
    assert_eq!((quiet.expired_events, quiet.tasks), (0, 0), "{quiet:?}");
    assert_eq!(t.tasks.completed().len(), 1);
}

/// AC4: a renewal that is accepted but already past its expiry date closes
/// no task and the check is still refused.
#[renox::test]
async fn a_renewal_already_expired_closes_nothing() {
    let w = world_with(
        boot_options(Options {
            owner: Some(OWNER),
            fake_events: false,
            ..Options::default()
        })
        .await,
        &[permit()],
    )
    .await;
    let t = &w.t;
    ravi_permit(&w).await;
    expiry::run(t.state(), day(31)).await.unwrap();

    let renewal = t
        .accepted(&w.rita, ravi(), "work_permit", Some(day(-1)))
        .await;

    assert_eq!(renewal.status, DocumentStatus::Accepted);
    assert!(
        t.accepted_events()
            .iter()
            .any(|e| e.document == renewal.document_id())
    );
    let row = open_task(t, "work_permit").await.expect("still open");
    assert_eq!(row.renewal_document_id, None);
    assert!(t.tasks.completed().is_empty());
    assert!(!may_resume(t, day(0)).await, "still refused");
}

/// AC5: a kind that is not compliance-critical tells the responsible staff
/// and not the MD, nor HR (nobody is named in `also_notify`).
#[renox::test]
async fn a_non_critical_kind_does_not_tell_the_owner() {
    let w = world_with(owner_options().await, &[site_card()]).await;
    let t = &w.t;
    w.t.accepted(&w.rita, ravi(), "site_card", Some(day(30)))
        .await;

    let report = expiry::run(t.state(), day(31)).await.unwrap();

    assert_eq!(
        (report.expired_events, report.messages, report.tasks),
        (1, 1, 1),
        "{report:?}"
    );
    assert_eq!(t.notices.sent().len(), 1);
    assert_eq!(t.notices.to_user(w.pia).len(), 1);
    assert!(t.notices.to_user(w.mo).is_empty());
    assert!(t.notices.to_user(w.hal).is_empty());
    assert_eq!(t.tasks.opened()[0].assignee, Some(Assignee::user(w.pia)));
}

/// AC5: a document never uploaded emits nothing, and neither does one that
/// was superseded before it expired.
#[renox::test]
async fn nothing_is_emitted_for_a_missing_or_superseded_document() {
    let w = world().await;
    let t = &w.t;

    let none = expiry::run(t.state(), day(31)).await.unwrap();
    assert_eq!((none.expired, none.expired_events, none.tasks), (0, 0, 0));

    ravi_permit(&w).await;
    let renewal = t
        .accepted(&w.rita, ravi(), "work_permit", Some(day(365)))
        .await;
    let report = expiry::run(t.state(), day(31)).await.unwrap();
    assert_eq!(
        (
            report.scanned,
            report.expired,
            report.expired_events,
            report.messages
        ),
        (1, 0, 0, 0),
        "only the accepted renewal is scanned: {report:?}"
    );
    assert!(t.app.emitted::<DocumentExpired>().is_empty());
    assert!(t.notices.sent().is_empty());
    assert!(t.tasks.opened().is_empty());
    assert!(
        !ExpiredLog::is_logged(t.db(), renewal.id).await.unwrap(),
        "the renewal has not expired"
    );
}

/// AC5: without `[roles] owner` the company admin is told for a
/// compliance-critical kind, not the MD.
#[renox::test]
async fn without_an_owner_role_the_company_admin_is_told() {
    let w = world_with(boot().await, &[permit()]).await;
    let t = &w.t;
    let ada = t.staff("Ada", COMPANY_ADMIN_ROLE, None).await.id;
    ravi_permit(&w).await;

    let report = expiry::run(t.state(), day(31)).await.unwrap();

    assert_eq!(
        (report.expired_events, report.messages),
        (1, 3),
        "{report:?}"
    );
    assert_eq!(t.notices.to_user(ada).len(), 1);
    assert_eq!(t.notices.to_user(w.pia).len(), 1);
    assert_eq!(t.notices.to_user(w.hal).len(), 1);
    assert!(t.notices.to_user(w.mo).is_empty());
}

/// AC6: while a renewal task is open for the subject and kind, a later
/// expiry opens no second task.
#[renox::test]
async fn an_open_renewal_task_is_not_opened_twice() {
    let w = world().await;
    let t = &w.t;
    ravi_permit(&w).await;
    expiry::run(t.state(), day(31)).await.unwrap();
    assert_eq!(t.tasks.opened().len(), 1);

    // The renewal is accepted but is expired too (the listener does not run
    // with faked events, so the task stays open anyway).
    let renewal = t
        .accepted(&w.rita, ravi(), "work_permit", Some(day(-1)))
        .await;
    let report = expiry::run(t.state(), day(32)).await.unwrap();

    assert_eq!(
        (report.expired_events, report.tasks, report.failed),
        (1, 0, 0),
        "{report:?}"
    );
    let events = t.app.emitted::<DocumentExpired>();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].document, renewal.document_id());
    assert_eq!(t.tasks.opened().len(), 1);
    let kind = t.kind("work_permit").await;
    let rows = RenewalTask::for_subject(t.db(), "hr.employee", RAVI, kind.id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].closed_at.is_none());
}

/// AC6: without ticketing the event and the messages still go out and the
/// task row has no ticket.
#[renox::test]
async fn without_ticketing_the_expiry_is_still_told() {
    let w = world_with(
        boot_options(Options {
            owner: Some(OWNER),
            tickets: false,
            ..Options::default()
        })
        .await,
        &[permit()],
    )
    .await;
    let t = &w.t;
    ravi_permit(&w).await;

    let report = expiry::run(t.state(), day(31)).await.unwrap();

    assert_eq!(
        (report.expired_events, report.messages, report.failed),
        (1, 3, 0),
        "{report:?}"
    );
    assert_eq!(t.app.emitted::<DocumentExpired>().len(), 1);
    assert_eq!(t.notices.sent().len(), 3);
    assert!(t.tasks.opened().is_empty());
    let row = open_task(t, "work_permit").await.expect("the task row");
    assert_eq!(row.ticket_id, None);
    assert_eq!(row.ticket_number, None);
}

/// AC7: a failing `FutureWork` for one document emits `AutomationFailed`
/// for it and the other documents are handled; the next run emits the event
/// for the failed one.
#[renox::test]
async fn a_failing_provider_does_not_stop_the_other_documents() {
    let w = world().await;
    let t = &w.t;
    let ravis = ravi_permit(&w).await;
    let mona = t
        .accepted(
            &w.rita,
            Subject::new("hr.employee", 8),
            "work_permit",
            Some(day(30)),
        )
        .await;
    t.plan.fail_for(Some(RAVI));

    let report = expiry::run(t.state(), day(31)).await.unwrap();

    assert_eq!(
        (report.expired, report.expired_events, report.failed),
        (2, 1, 1),
        "{report:?}"
    );
    let failed = t.app.emitted::<AutomationFailed>();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].rule, "DOC.RUL.03");
    assert_eq!(failed[0].rule, expired::RULE);
    assert_eq!(failed[0].subject, renewal::document_subject(ravis.id));
    assert!(failed[0].compliance_critical);
    assert!(failed[0].error.contains("not available"), "{failed:?}");
    let events = t.app.emitted::<DocumentExpired>();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].document, mona.document_id());
    assert!(!ExpiredLog::is_logged(t.db(), ravis.id).await.unwrap());
    assert!(ExpiredLog::is_logged(t.db(), mona.id).await.unwrap());
    assert_eq!(t.tasks.opened().len(), 1);

    t.plan.fail_for(None);
    let next = expiry::run(t.state(), day(32)).await.unwrap();

    assert_eq!((next.expired_events, next.failed), (1, 0), "{next:?}");
    let events = t.app.emitted::<DocumentExpired>();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].document, ravis.document_id());
    assert_eq!(events[1].blocked.len(), 2);
    assert_eq!(t.app.emitted::<AutomationFailed>().len(), 1);
    assert_eq!(t.tasks.opened().len(), 2);
}
