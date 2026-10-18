//! The possible-match alerts and the screening task pages (wl-parties §4,
//! issue #38 part 4): the bell entry for the compliance role only, and
//! `/parties/screening-tasks` behind `parties.block_reason.view`.

mod support;

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::notify::{self, Notifier, NotifyRequest};
use wl_parties::{
    NewCandidate, ScreeningRun, ScreeningTask, ScreeningTrigger, WlParties, permissions as perm,
    record_run,
};

use support::{Booted, boot_custom};

const AL_NOOR: &str = "Al Noor Trading LLC";

#[derive(Clone, Default)]
struct Recording(Arc<Mutex<Vec<NotifyRequest>>>);

impl Notifier for Recording {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> notify::NotifyFuture<'a, Vec<notify::MessageId>> {
        let count = request.recipients.len() as i64;
        self.0.lock().unwrap().push(request);
        Box::pin(async move { Ok((1..=count).map(notify::MessageId).collect()) })
    }
}

struct WithNotifier(Recording);

impl Module for WithNotifier {
    fn name(&self) -> &'static str {
        "test-notifier"
    }

    fn register(&self, app: &mut Registry) {
        notify::provide_notifier(app, self.0.clone());
    }
}

fn flagged_run() -> ScreeningRun {
    ScreeningRun::automatic(
        ScreeningTrigger::Onboarding,
        vec![NewCandidate {
            name: "Omar K.".into(),
            list_name: "UN consolidated".into(),
            score: 0.91,
            list_entry: Some("QDi.001".into()),
        }],
    )
}

async fn booted(recording: Option<Recording>) -> Booted {
    boot_custom(
        WlParties::new().screening(true),
        true,
        |app| match recording {
            Some(r) => app.module(WithNotifier(r)),
            None => app,
        },
    )
    .await
}

#[renox::test]
async fn a_flag_alerts_every_holder_once_and_no_one_else() {
    let recording = Recording::default();
    let t = booted(Some(recording.clone())).await;
    let party = t.party(AL_NOOR).await;
    let mlro = t.mlro().await;
    let deputy = t.user("Dina", &[perm::VIEW, perm::BLOCK_REASON_VIEW]).await;
    let partner = t.user("Pia", &[perm::VIEW]).await;

    record_run(t.app.state(), &party, flagged_run(), None)
        .await
        .unwrap();
    let task = ScreeningTask::open_for_party(t.db(), party.id)
        .await
        .unwrap()
        .expect("a task");

    for holder in [&mlro, &deputy] {
        let bells = holder.unread_notifications(t.db()).await.unwrap();
        assert_eq!(bells.len(), 1);
        let message = bells[0].message().expect("a message");
        assert_eq!(message.title, "A possible sanctions match needs review");
        let body = message.body.clone().unwrap_or_default();
        assert!(body.contains(&party.code), "{body}");
        assert!(!body.contains("Omar") && !body.contains("UN consolidated"));
    }
    assert!(
        partner
            .unread_notifications(t.db())
            .await
            .unwrap()
            .is_empty()
    );

    let requests = recording.0.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert_eq!(request.type_key, "party.screening.possible_match");
        assert_eq!(request.recipients.len(), 1);
        let data = request.data.to_string();
        assert!(data.contains(&party.code) && !data.contains("Omar"));
    }

    // A re-screen linked to the open task alerts no one again.
    record_run(t.app.state(), &party, flagged_run(), None)
        .await
        .unwrap();
    assert_eq!(mlro.unread_notifications(t.db()).await.unwrap().len(), 1);
    assert_eq!(recording.0.lock().unwrap().len(), 2);
    assert_eq!(task.status, wl_parties::ScreeningTaskStatus::Open);
}

#[renox::test]
async fn without_notify_only_the_bell_is_sent() {
    let t = booted(None).await;
    let party = t.party(AL_NOOR).await;
    let mlro = t.mlro().await;
    record_run(t.app.state(), &party, flagged_run(), None)
        .await
        .unwrap();
    assert_eq!(mlro.unread_notifications(t.db()).await.unwrap().len(), 1);
}

#[renox::test]
async fn the_pages_show_the_task_to_holders_and_are_404_for_others() {
    let t = booted(None).await;
    let party = t.party(AL_NOOR).await;
    let mlro = t.mlro().await;
    let partner = t.user("Pia", &[perm::VIEW]).await;
    record_run(t.app.state(), &party, flagged_run(), None)
        .await
        .unwrap();
    let task = ScreeningTask::open_for_party(t.db(), party.id)
        .await
        .unwrap()
        .expect("a task");
    let detail = format!("/parties/screening-tasks/{}", task.id);

    t.app.acting_as(&mlro);
    let list = t.app.get("/parties/screening-tasks").await;
    list.assert_ok().assert_see(&detail);
    let page = t.app.get(&detail).await;
    page.assert_ok()
        .assert_see("Omar K.")
        .assert_see("UN consolidated")
        .assert_see("QDi.001")
        .assert_see("0.91")
        .assert_see(&party.code);
    t.app
        .get("/parties/screening-tasks/99999")
        .await
        .assert_not_found();

    t.app.acting_as(&partner);
    t.app
        .get("/parties/screening-tasks")
        .await
        .assert_not_found();
    t.app.get(&detail).await.assert_not_found();
    t.app
        .get("/parties/screening-tasks/99999")
        .await
        .assert_not_found();
}
