//! Acceptance checks of listing the planned work an expiring document
//! affects (#48, AC1-AC6): the nightly `expiry::run`, the recording
//! notifier and the planned fire-safety visits of `support`. Dates are
//! relative to today: the expiry date is `day(30)`, so `day(0)` is the night
//! the 30-day window is reached.

mod support;

use wl_documents::expiry::{self, AffectedItem, ReminderLog};
use wl_documents::{Document, DocumentStatus, KindSeed};
use wl_kernel::documents::DocumentExpiring;

use support::{
    Booted, boot, boot_with, civil_defence_approval, day, ecas_certificate, plant1, plant2,
    safefire, tower_a, tower_b, valve,
};

/// The app with SafeFire's approval (made from `kind`) expiring in 30 days
/// and one coordinator in each tower.
struct Towers {
    t: Booted,
    document: Document,
    anna: i64,
    bela: i64,
    rita: renox::prelude::User,
}

async fn towers_with(t: Booted, kind: KindSeed) -> Towers {
    t.install(&[kind]).await;
    t.roles(&["coordinator"]).await;
    let anna = t.staff("Anna", "coordinator", Some(&tower_a())).await.id;
    let bela = t.staff("Bela", "coordinator", Some(&tower_b())).await.id;
    let rita = t.reviewer().await;
    let document = t
        .accepted(&rita, safefire(), "civil_defence_approval", Some(day(30)))
        .await;
    Towers {
        t,
        document,
        anna,
        bela,
        rita,
    }
}

async fn towers() -> Towers {
    towers_with(boot().await, civil_defence_approval()).await
}

/// The dates listed in a message, in order.
fn dates(request: &wl_kernel::notify::NotifyRequest) -> Vec<String> {
    request.data["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["date"].as_str().unwrap().to_owned())
        .collect()
}

/// AC1: the two later visits go to Tower A's coordinator, once; the visit
/// on the expiry date is not listed and Tower B hears nothing.
#[renox::test]
async fn later_visits_are_reported_to_the_coordinator_of_the_building() {
    let w = towers().await;
    let t = &w.t;
    t.plan.visit(tower_a(), day(35));
    t.plan.visit(tower_a(), day(60));
    // On the expiry date the document is still valid.
    t.plan.visit(tower_a(), day(30));

    let report = expiry::run(t.state(), day(0)).await.unwrap();

    assert_eq!(
        (
            report.scanned,
            report.events,
            report.messages,
            report.failed
        ),
        (1, 1, 1, 0),
        "{report:?}"
    );
    assert_eq!(t.notices.sent().len(), 1);
    assert!(t.notices.to_user(w.bela).is_empty());
    let sent = t.notices.to_user(w.anna);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].type_key, expiry::run::NOTIFICATION_TYPE);
    assert_eq!(dates(&sent[0]), [day(35).to_string(), day(60).to_string()]);
    let events = t.app.emitted::<DocumentExpiring>();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].document_id, w.document.document_id());
    assert_eq!(events[0].days_left, 30);
    assert_eq!(events[0].affected.len(), 1);
    assert_eq!(events[0].affected[0].guarded, "ticket.resolve");
    assert_eq!(events[0].affected[0].records.len(), 2);
}

/// AC2: a night with nothing new is silent; a later visit is the only one
/// listed, while the event holds all; the same night twice adds nothing.
#[renox::test]
async fn only_new_visits_are_reported_and_a_night_runs_once() {
    let w = towers().await;
    let t = &w.t;
    t.plan.visit(tower_a(), day(35));
    t.plan.visit(tower_a(), day(60));
    expiry::run(t.state(), day(0)).await.unwrap();
    t.notices.clear();

    let quiet = expiry::run(t.state(), day(1)).await.unwrap();
    assert_eq!((quiet.events, quiet.messages), (0, 0), "{quiet:?}");
    assert!(t.notices.sent().is_empty());
    assert_eq!(t.app.emitted::<DocumentExpiring>().len(), 1);

    t.plan.visit(tower_a(), day(58));
    let third = expiry::run(t.state(), day(2)).await.unwrap();
    assert_eq!((third.events, third.messages), (1, 1), "{third:?}");
    let sent = t.notices.to_user(w.anna);
    assert_eq!(t.notices.sent().len(), 1);
    assert_eq!(dates(&sent[0]), [day(58).to_string()]);
    let events = t.app.emitted::<DocumentExpiring>();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].days_left, 28);
    assert_eq!(events[1].affected[0].records.len(), 3);

    t.notices.clear();
    let again = expiry::run(t.state(), day(2)).await.unwrap();
    assert_eq!((again.events, again.messages), (0, 0), "{again:?}");
    assert!(t.notices.sent().is_empty());
    assert_eq!(t.app.emitted::<DocumentExpiring>().len(), 2);
}

/// AC3: with a look-ahead of 45 days the window opens 45 days before.
#[renox::test]
async fn the_look_ahead_of_the_kind_opens_the_window() {
    let w = towers_with(boot().await, civil_defence_approval().look_ahead(45)).await;
    let t = &w.t;
    t.plan.visit(tower_a(), day(35));

    let before = expiry::run(t.state(), day(-16)).await.unwrap();
    assert_eq!((before.events, before.messages), (0, 0), "{before:?}");
    assert!(t.notices.sent().is_empty());

    let opened = expiry::run(t.state(), day(-15)).await.unwrap();
    assert_eq!((opened.events, opened.messages), (1, 1), "{opened:?}");
    assert_eq!(t.notices.to_user(w.anna).len(), 1);
    assert_eq!(t.app.emitted::<DocumentExpiring>()[0].days_left, 45);
}

/// AC4: the 30-day reminder of a certificate with no scope goes to the QC
/// inspectors of the plant the schedule names, once.
#[renox::test]
async fn the_reminder_goes_to_the_plant_the_schedule_names() {
    let t = boot().await;
    t.install(&[ecas_certificate()]).await;
    t.roles(&["qc_inspector"]).await;
    let pia = t.staff("Pia", "qc_inspector", Some(&plant1())).await;
    let quin = t.staff("Quin", "qc_inspector", Some(&plant2())).await;
    let rita = t.reviewer().await;
    let document = t
        .accepted(&rita, valve(), "ecas_certificate", Some(day(30)))
        .await;
    t.plan.schedule(vec![plant1()]);

    let report = expiry::run(t.state(), day(0)).await.unwrap();

    assert_eq!((report.events, report.messages), (1, 1), "{report:?}");
    let sent = t.notices.to_user(pia.id);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].data["offset"], 30);
    assert!(t.notices.to_user(quin.id).is_empty());
    let events = t.app.emitted::<DocumentExpiring>();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].offset, Some(30));
    assert!(events[0].affected.is_empty());
    assert!(t.plan.visits().is_empty());
    assert!(
        ReminderLog::offsets_for(t.db(), document.id)
            .await
            .unwrap()
            .contains(&30)
    );

    // A second run that night sends nothing.
    t.notices.clear();
    let again = expiry::run(t.state(), day(0)).await.unwrap();
    assert_eq!((again.events, again.messages), (0, 0), "{again:?}");
    assert!(t.notices.sent().is_empty());
}

/// AC4: without a `ReminderSchedule` the QC inspectors of every plant hear.
#[renox::test]
async fn without_a_schedule_the_reminder_goes_to_every_plant() {
    let t = boot().await;
    t.install(&[ecas_certificate()]).await;
    t.roles(&["qc_inspector"]).await;
    let pia = t.staff("Pia", "qc_inspector", Some(&plant1())).await;
    let quin = t.staff("Quin", "qc_inspector", Some(&plant2())).await;
    let rita = t.reviewer().await;
    t.accepted(&rita, valve(), "ecas_certificate", Some(day(30)))
        .await;

    expiry::run(t.state(), day(0)).await.unwrap();

    assert_eq!(t.notices.to_user(pia.id).len(), 1);
    assert_eq!(t.notices.to_user(quin.id).len(), 1);
    assert_eq!(t.notices.sent().len(), 2);
}

/// AC5: a renewal accepted before the run supersedes the old approval: its
/// visits are not reported and keep their state.
#[renox::test]
async fn a_renewed_approval_reports_nothing() {
    let w = towers().await;
    let t = &w.t;
    t.plan.visit(tower_a(), day(35));
    t.plan.visit(tower_a(), day(60));
    let before = t.plan.visits().len();
    let renewal = t
        .accepted(
            &w.rita,
            safefire(),
            "civil_defence_approval",
            Some(day(365)),
        )
        .await;
    assert_eq!(renewal.status, DocumentStatus::Accepted);
    assert_eq!(
        t.reload(&w.document).await.status,
        DocumentStatus::Superseded
    );

    let report = expiry::run(t.state(), day(0)).await.unwrap();

    assert_eq!(report.scanned, 1, "only the accepted renewal is scanned");
    assert_eq!(
        (report.events, report.messages, report.failed),
        (0, 0, 0),
        "{report:?}"
    );
    assert!(t.notices.sent().is_empty());
    assert!(t.app.emitted::<DocumentExpiring>().is_empty());
    assert!(
        AffectedItem::for_document(t.db(), w.document.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(t.plan.visits().len(), before);
}

/// AC5: a blocking document with no affected items and no offset reached
/// emits no event.
#[renox::test]
async fn a_blocking_document_with_nothing_planned_is_silent() {
    let w = towers().await;
    let t = &w.t;
    // Only work on or before the expiry date, and in a category the entry
    // does not guard, is planned.
    t.plan.visit(tower_a(), day(30));
    t.plan.visit(tower_a(), day(10));

    let report = expiry::run(t.state(), day(0)).await.unwrap();

    assert_eq!(
        (report.scanned, report.events, report.messages),
        (1, 0, 0),
        "{report:?}"
    );
    assert!(t.app.emitted::<DocumentExpiring>().is_empty());
    assert!(t.notices.sent().is_empty());
    assert_eq!(t.plan.visits().len(), 2);
}

/// AC6: the task is the daily 01:00 Asia/Dubai run and does the check for
/// today.
#[renox::test]
async fn the_task_runs_the_check_for_today() {
    assert_eq!(expiry::run::TASK, "documents:expiry");
    assert_eq!(expiry::run::TASK_AT, "01:00");
    assert_eq!(expiry::run::TASK_ZONE, "Asia/Dubai");

    let w = towers().await;
    let t = &w.t;
    t.plan.visit(tower_a(), day(35));
    t.app
        .kernel()
        .run_scheduled(expiry::run::TASK)
        .await
        .expect("the task runs");

    assert_eq!(t.notices.to_user(w.anna).len(), 1);
    assert!(t.notices.to_user(w.bela).is_empty());
    assert_eq!(t.app.emitted::<DocumentExpiring>().len(), 1);
}

/// AC6: `documents:expiry --date` runs the same check for that day.
#[renox::test]
async fn the_command_checks_the_given_date() {
    let w = towers().await;
    let t = &w.t;
    t.plan.visit(tower_a(), day(35));

    // 31 days left: outside the window.
    let early = day(-1).to_string();
    t.app
        .kernel()
        .call(expiry::run::TASK, ["--date", early.as_str()])
        .await
        .expect("the command runs");
    assert!(t.notices.sent().is_empty());

    let date = day(0).to_string();
    t.app
        .kernel()
        .call(expiry::run::TASK, [format!("--date={date}")])
        .await
        .expect("the command runs");
    assert_eq!(t.notices.to_user(w.anna).len(), 1);
    assert_eq!(t.app.emitted::<DocumentExpiring>().len(), 1);

    let refused = t
        .app
        .kernel()
        .call(expiry::run::TASK, ["--date", "tomorrow"])
        .await;
    assert!(refused.is_err());
}

/// AC6: without wl-notify the run still logs the items and emits the event.
#[renox::test]
async fn without_wl_notify_the_run_still_logs_and_emits() {
    let w = towers_with(boot_with(false).await, civil_defence_approval()).await;
    let t = &w.t;
    t.plan.visit(tower_a(), day(35));
    t.plan.visit(tower_a(), day(60));

    let report = expiry::run(t.state(), day(0)).await.unwrap();

    assert_eq!((report.events, report.failed), (1, 0), "{report:?}");
    assert!(t.notices.sent().is_empty());
    assert_eq!(
        AffectedItem::for_document(t.db(), w.document.id)
            .await
            .unwrap()
            .len(),
        2
    );
    let events = t.app.emitted::<DocumentExpiring>();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].affected[0].records.len(), 2);
}
