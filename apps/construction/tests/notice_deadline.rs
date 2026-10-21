//! BL-CON-14 (#293): `CON.RUL.10` warns the project manager and QS 7 and 2
//! days before the contract's notice deadline (the event date plus the
//! project's notice period) while the variation for the event is a draft; a
//! notice sent late is marked with its date and never refused.

mod support;

use construction::notice_deadline;
use construction::variation::{self, Variation};
use construction::{Project, ProjectStatus, VariationStatus};
use renox::prelude::*;

use support::{Booted, boot, date, users_of};

fn path(variation: &Variation, step: &str) -> String {
    format!("/construction/variations/{}/{step}", variation.id)
}

/// The QS records a time variation for the delay event of `event_date`.
async fn record_time(t: &Booted, qs: &User, project: &Project, event_date: &str) -> Variation {
    t.post(
        qs,
        &format!("/construction/projects/{}/variations", project.id),
        &[
            ("kind", "time"),
            ("description", "Extension of time"),
            ("days_asked", "30"),
            ("event", "late drawings"),
            ("event_date", event_date),
        ],
    )
    .await
    .assert_ok();
    variation::of_project(t.db(), project.id)
        .await
        .unwrap()
        .pop()
        .expect("the variation")
}

async fn set_notice_days(t: &Booted, project: &Project, days: Option<i64>) {
    Project::query()
        .where_(Project::ID.eq(project.id))
        .update(t.db(), &[("notice_days", &days)])
        .await
        .unwrap();
}

/// Done when 1: a notice period of 28 days and a draft time variation with
/// event date 2027-04-01 (deadline 2027-04-29): the project manager and QS are
/// warned on 2027-04-22 and 2027-04-27, once each, and not on other days.
#[renox::test]
async fn the_project_manager_and_qs_are_warned_seven_and_two_days_before_the_deadline() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let project = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    set_notice_days(&t, &project, Some(28)).await;
    let draft = record_time(&t, &qs, &project, "2027-04-01").await;
    assert_eq!(draft.status, VariationStatus::Draft);

    t.on(date(2027, 4, 21), async {
        let run = notice_deadline::run_on(t.state(), date(2027, 4, 21))
            .await
            .unwrap();
        assert!(run.first.is_empty() && run.second.is_empty());
    })
    .await;
    assert!(t.notices.notices_titled("Notice for").is_empty());

    t.on(date(2027, 4, 22), async {
        notice_deadline::daily(t.state()).await.unwrap();
        notice_deadline::daily(t.state()).await.unwrap();
    })
    .await;
    let first = t.notices.notices_titled("Notice for");
    assert_eq!(first.len(), 1);
    let mut users = users_of(&first[0]);
    users.sort_unstable();
    let mut expected = vec![pm.id, qs.id];
    expected.sort_unstable();
    assert_eq!(users, expected);
    assert!(first[0].data["title"].as_str().unwrap().contains("7 days"));
    assert!(
        first[0].data["body"]
            .as_str()
            .unwrap()
            .contains("2027-04-29")
    );

    t.on(date(2027, 4, 24), async {
        notice_deadline::daily(t.state()).await.unwrap();
    })
    .await;
    assert_eq!(t.notices.notices_titled("Notice for").len(), 1);

    t.on(date(2027, 4, 27), async {
        notice_deadline::daily(t.state()).await.unwrap();
        notice_deadline::daily(t.state()).await.unwrap();
    })
    .await;
    let all = t.notices.notices_titled("Notice for");
    assert_eq!(all.len(), 2);
    assert!(all[1].data["title"].as_str().unwrap().contains("2 days"));
    assert_eq!(users_of(&all[1]).len(), 2);

    // After the deadline nothing more is sent.
    t.on(date(2027, 4, 30), async {
        notice_deadline::daily(t.state()).await.unwrap();
    })
    .await;
    assert_eq!(t.notices.notices_titled("Notice for").len(), 2);
}

/// Done when 2: submitted on 2027-05-02, after the deadline, the variation is
/// marked "notice late" with notice date 2027-05-02 (the submission is not
/// refused); one submitted on the deadline is not late.
#[renox::test]
async fn a_notice_after_the_deadline_is_marked_late_and_not_refused() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let project = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    set_notice_days(&t, &project, Some(28)).await;
    let on_time = record_time(&t, &qs, &project, "2027-04-01").await;
    let late = record_time(&t, &qs, &project, "2027-04-01").await;

    t.on(date(2027, 4, 29), async {
        t.post(&pm, &path(&on_time, "submit"), &[])
            .await
            .assert_ok();
    })
    .await;
    let sent = t.reload_variation(on_time.id).await;
    assert_eq!(sent.status, VariationStatus::Submitted);
    assert_eq!(sent.notice_date, Some(date(2027, 4, 29)));
    assert!(!sent.notice_late);

    t.on(date(2027, 5, 2), async {
        t.post(&pm, &path(&late, "submit"), &[]).await.assert_ok();
    })
    .await;
    let sent = t.reload_variation(late.id).await;
    assert_eq!(sent.status, VariationStatus::Submitted);
    assert_eq!(sent.notice_date, Some(date(2027, 5, 2)));
    assert!(sent.notice_late);
    let report = variation::report_of(t.db(), project.id).await.unwrap();
    assert!(
        report
            .iter()
            .any(|line| line.id == late.id && line.notice_late)
    );
}

/// Done when 3: with no notice period set there is no warning and no late
/// marker; a variation without an event date is skipped.
#[renox::test]
async fn without_a_notice_period_or_an_event_date_there_is_no_warning() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let none = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    let draft = record_time(&t, &qs, &none, "2027-04-01").await;
    let with_period = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    set_notice_days(&t, &with_period, Some(28)).await;
    // An addition without an event date.
    t.post(
        &qs,
        &format!("/construction/projects/{}/variations", with_period.id),
        &[
            ("kind", "addition"),
            ("description", "Extra door"),
            ("priced_amount", "1000000"),
        ],
    )
    .await
    .assert_ok();

    t.on(date(2027, 4, 27), async {
        let run = notice_deadline::run_on(t.state(), date(2027, 4, 27))
            .await
            .unwrap();
        assert!(run.first.is_empty() && run.second.is_empty());
    })
    .await;
    assert!(t.notices.notices_titled("Notice for").is_empty());

    t.on(date(2027, 5, 2), async {
        t.post(&pm, &path(&draft, "submit"), &[]).await.assert_ok();
    })
    .await;
    let sent = t.reload_variation(draft.id).await;
    assert_eq!(sent.notice_date, Some(date(2027, 5, 2)));
    assert!(!sent.notice_late);
}
