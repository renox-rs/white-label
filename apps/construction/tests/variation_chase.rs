//! BL-CON-13 (#292): `CON.RUL.09` chases a variation the client does not
//! answer. Daily at 02:00 Asia/Dubai, 14 days after the latest `submit` the
//! project manager is reminded to chase the client and at 30 days the MD is
//! notified, once per submission; an answered or withdrawn variation is not
//! chased, and a resubmitted revision has its own clock.

mod support;

use construction::chase;
use construction::variation::{self, Variation};
use construction::{Project, ProjectStatus, VariationStatus};
use renox::prelude::*;

use support::{Booted, boot, date, users_of};

fn path(variation: &Variation, step: &str) -> String {
    format!("/construction/variations/{}/{step}", variation.id)
}

/// The QS records a priced variation of the project.
async fn record(t: &Booted, qs: &User, project: &Project, description: &str) -> Variation {
    t.post(
        qs,
        &format!("/construction/projects/{}/variations", project.id),
        &[
            ("kind", "addition"),
            ("description", description),
            ("priced_amount", "1000000"),
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

/// Done when 1 and 2: VO submitted 2027-03-01 and not answered, the project
/// manager is reminded on 2027-03-15 and the MD is notified on 2027-03-31; one
/// approved on 2027-03-20 gets no MD notice. A withdrawn variation is never
/// chased, and a resubmitted revision has its own clock from its own `submit`.
#[renox::test]
async fn an_unanswered_variation_is_chased_after_fourteen_days_and_escalated_after_thirty() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let md = t.md("Omar").await;
    let project = t.project(ProjectStatus::InProgress, Some(&pm)).await;

    // Four variations go to the client on 2027-03-01.
    let (open, answered, withdrawn, rejected) = t
        .on(date(2027, 3, 1), async {
            let open = record(&t, &qs, &project, "Unanswered partition").await;
            let answered = record(&t, &qs, &project, "Answered door").await;
            let withdrawn = record(&t, &qs, &project, "Withdrawn ceiling").await;
            let rejected = record(&t, &qs, &project, "Rejected cladding").await;
            for made in [&open, &answered, &withdrawn, &rejected] {
                t.post(&pm, &path(made, "submit"), &[]).await.assert_ok();
            }
            (open, answered, withdrawn, rejected)
        })
        .await;
    assert_eq!(
        t.reload_variation(open.id).await.submitted_on,
        Some(date(2027, 3, 1))
    );
    assert!(t.notices.notices_titled("Chase the client").is_empty());

    t.on(date(2027, 3, 3), async {
        t.post(&pm, &path(&withdrawn, "withdraw"), &[])
            .await
            .assert_ok();
    })
    .await;
    // The client rejects one and the project manager resubmits it: revision 2
    // goes to the client on 2027-03-12, its own clock.
    let second = t
        .on(date(2027, 3, 12), async {
            t.post(
                &qs,
                &path(&rejected, "reject"),
                &[("reason", "not instructed")],
            )
            .await
            .assert_ok();
            t.post(&pm, &path(&rejected, "resubmit"), &[])
                .await
                .assert_ok();
            let second = variation::revisions_of(t.db(), project.id, &rejected.code)
                .await
                .unwrap()
                .pop()
                .unwrap();
            assert_eq!(second.revision, 2);
            t.post(&pm, &path(&second, "submit"), &[]).await.assert_ok();
            second
        })
        .await;
    assert_eq!(
        t.reload_variation(second.id).await.submitted_on,
        Some(date(2027, 3, 12))
    );

    // Day 13: nothing.
    t.on(date(2027, 3, 14), async {
        let run = chase::run_on(t.state(), date(2027, 3, 14)).await.unwrap();
        assert!(run.reminded.is_empty());
        assert!(run.escalated.is_empty());
    })
    .await;
    assert!(t.notices.notices_titled("Chase the client").is_empty());

    // Day 14: the project manager is reminded for the two still unanswered.
    t.on(date(2027, 3, 15), async {
        chase::daily(t.state()).await.unwrap();
        // The same day again, or a later run, makes nothing new.
        chase::daily(t.state()).await.unwrap();
    })
    .await;
    let reminders = t.notices.notices_titled("Chase the client");
    assert_eq!(reminders.len(), 2, "not the withdrawn nor the rejected one");
    for reminder in &reminders {
        assert_eq!(users_of(reminder), [pm.id]);
        assert!(reminder.data["title"].as_str().unwrap().contains("14 days"));
        assert!(
            reminder.data["body"]
                .as_str()
                .unwrap()
                .contains("2027-03-01")
        );
    }
    let titles: Vec<&str> = reminders
        .iter()
        .map(|reminder| reminder.data["title"].as_str().unwrap())
        .collect();
    assert!(titles.iter().any(|title| title.contains("VO-01 rev 1")));
    assert!(titles.iter().any(|title| title.contains("VO-02 rev 1")));
    assert_eq!(
        t.reload_variation(open.id).await.chased_on,
        Some(date(2027, 3, 15))
    );
    assert_eq!(t.reload_variation(withdrawn.id).await.chased_on, None);
    assert!(
        t.notices
            .notices_titled("no answer from the client")
            .is_empty()
    );

    // The client approves one on 2027-03-20; the revision's day 14 is the 26th.
    t.on(date(2027, 3, 20), async {
        t.post(
            &qs,
            &path(&answered, "approve"),
            &[("client_reference", "CL-9"), ("approved_amount", "900000")],
        )
        .await
        .assert_ok();
    })
    .await;
    assert_eq!(
        t.reload_variation(answered.id).await.status,
        VariationStatus::Approved
    );
    t.on(date(2027, 3, 26), async {
        chase::daily(t.state()).await.unwrap();
    })
    .await;
    let reminders = t.notices.notices_titled("Chase the client");
    assert_eq!(reminders.len(), 3);
    assert!(
        reminders[2].data["title"]
            .as_str()
            .unwrap()
            .contains("VO-04 rev 2")
    );

    // Day 30: the MD is told about the one the client never answered, not the
    // approved one.
    t.on(date(2027, 3, 31), async {
        chase::daily(t.state()).await.unwrap();
        chase::daily(t.state()).await.unwrap();
    })
    .await;
    let escalated = t.notices.notices_titled("no answer from the client");
    assert_eq!(escalated.len(), 1);
    assert_eq!(users_of(&escalated[0]), [md.id]);
    let title = escalated[0].data["title"].as_str().unwrap();
    assert!(
        title.contains("VO-01 rev 1") && title.contains("30 days"),
        "{title}"
    );
    assert_eq!(
        t.reload_variation(open.id).await.escalated_on,
        Some(date(2027, 3, 31))
    );
    assert_eq!(t.reload_variation(answered.id).await.escalated_on, None);

    // The revision's own day 30 is 2027-04-11.
    t.on(date(2027, 4, 10), async {
        chase::daily(t.state()).await.unwrap();
    })
    .await;
    assert_eq!(
        t.notices.notices_titled("no answer from the client").len(),
        1
    );
    t.on(date(2027, 4, 11), async {
        chase::daily(t.state()).await.unwrap();
    })
    .await;
    let escalated = t.notices.notices_titled("no answer from the client");
    assert_eq!(escalated.len(), 2);
    assert!(
        escalated[1].data["title"]
            .as_str()
            .unwrap()
            .contains("VO-04 rev 2")
    );
}

/// A run that missed the 14th day makes the reminder up, and a variation
/// resubmitted after an answer is chased again from its own `submit`.
#[renox::test]
async fn a_missed_day_is_made_up_and_a_new_submission_starts_again() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let _md = t.md("Omar").await;
    let project = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    let made = t
        .on(date(2027, 5, 3), async {
            let made = record(&t, &qs, &project, "Late partition").await;
            t.post(&pm, &path(&made, "submit"), &[]).await.assert_ok();
            made
        })
        .await;
    // The run of the 17th, the 14th day, was missed: the next one makes it up.
    t.on(date(2027, 5, 18), async {
        let run = chase::run_on(t.state(), date(2027, 5, 18)).await.unwrap();
        assert_eq!(run.reminded.len(), 1);
        assert!(run.escalated.is_empty());
    })
    .await;
    // Withdrawn after the reminder: nothing more is sent.
    t.on(date(2027, 5, 20), async {
        t.post(&pm, &path(&made, "withdraw"), &[]).await.assert_ok();
    })
    .await;
    t.on(date(2027, 6, 5), async {
        let run = chase::run_on(t.state(), date(2027, 6, 5)).await.unwrap();
        assert!(run.reminded.is_empty());
        assert!(run.escalated.is_empty());
    })
    .await;
    assert!(
        t.notices
            .notices_titled("no answer from the client")
            .is_empty()
    );
    assert_eq!(t.notices.notices_titled("Chase the client").len(), 1);
}
