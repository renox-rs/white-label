//! Sending a possible sanctions match to the compliance role only (wl-parties
//! §4, `PTY.SCR.01`): the acceptance checks AC2–AC5 and AC7 of #38, with the
//! clock moved to a fixed day, a partner user, the MLRO and the deputy, and
//! a fake mailer. AC1, AC6 and AC8 are in `screening.rs`.

mod support;

use std::sync::{Arc, Mutex};

use renox::chrono::{TimeZone, Utc};
use renox::prelude::*;
use wl_kernel::PartyStatus;
use wl_kernel::notify::NotifyChannel;
use wl_notify::channels::ProviderFuture;
use wl_notify::{ChannelProvider, Message, Outgoing, SendError, WlNotify};
use wl_parties::notify::POSSIBLE_MATCH_TYPE;
use wl_parties::{
    NewCandidate, Party, ScreeningRun, ScreeningTask, ScreeningTrigger, WlParties,
    permissions as perm, record_run,
};

use support::{Booted, boot_custom};

const AL_NOOR: &str = "Al Noor Trading LLC";

/// A fake e-mail provider that keeps what it was given.
#[derive(Clone, Default)]
struct Mailer(Arc<Mutex<Vec<Outgoing>>>);

impl ChannelProvider for Mailer {
    fn name(&self) -> &'static str {
        "test-mailer"
    }

    fn channel(&self) -> NotifyChannel {
        NotifyChannel::Email
    }

    fn available(&self, _state: &AppState) -> bool {
        true
    }

    fn send<'a>(
        &'a self,
        _state: &'a AppState,
        outgoing: &'a Outgoing,
    ) -> ProviderFuture<'a, std::result::Result<String, SendError>> {
        self.0.lock().unwrap().push(outgoing.clone());
        let id = self.0.lock().unwrap().len();
        Box::pin(async move { Ok(format!("mail-{id}")) })
    }
}

fn candidate(name: &str, score: f64) -> NewCandidate {
    NewCandidate {
        name: name.into(),
        list_name: "UN consolidated".into(),
        score,
        list_entry: Some("QDi.001".into()),
    }
}

fn run(candidates: Vec<NewCandidate>) -> ScreeningRun {
    ScreeningRun::automatic(ScreeningTrigger::Review, candidates)
}

/// Everything AC2–AC7 needs: the app with screening and wl-notify, the
/// MLRO, the deputy and a partner with `parties.view` only.
struct World {
    t: Booted,
    mailer: Mailer,
    party: Party,
    mlro: User,
    deputy: User,
    partner: User,
}

/// Moves the clock to 2026-11-03 `hour:00:10` Asia/Dubai (UTC+4), a few
/// seconds in so the minute shown does not roll over while a test runs.
fn set_clock(t: &Booted, dubai_hour: u32) {
    t.app.travel_back();
    let wanted = Utc
        .with_ymd_and_hms(2026, 11, 3, dubai_hour - 4, 0, 10)
        .unwrap();
    let ahead = (wanted - Utc::now()).num_seconds();
    t.app
        .travel(std::time::Duration::from_secs(ahead.unsigned_abs()));
    assert!(ahead > 0, "the fixed day is in the future");
}

async fn world() -> World {
    let mailer = Mailer::default();
    let provider = mailer.clone();
    let t = boot_custom(WlParties::new().screening(true), true, move |app| {
        app.module(WlNotify::new().provider(provider))
    })
    .await;
    t.app
        .kernel()
        .call("notify:install", Vec::<String>::new())
        .await
        .expect("notify:install runs");
    set_clock(&t, 14);
    let party = t.party(AL_NOOR).await;
    let mlro = t.mlro().await;
    let deputy = t.user("Dina", &[perm::VIEW, perm::BLOCK_REASON_VIEW]).await;
    let partner = t.user("Pia", &[perm::VIEW, perm::EXPORT]).await;
    World {
        t,
        mailer,
        party,
        mlro,
        deputy,
        partner,
    }
}

impl World {
    /// A run recorded at the fixed clock, then the queue worked off.
    async fn screen(&self, candidates: Vec<NewCandidate>) {
        self.t
            .app
            .at_travelled_time(record_run(
                self.t.app.state(),
                &self.party,
                run(candidates),
                None,
            ))
            .await
            .unwrap();
        self.t.app.run_all_jobs().await;
    }

    async fn tasks(&self) -> Vec<ScreeningTask> {
        ScreeningTask::query()
            .where_(ScreeningTask::PARTY_ID.eq(self.party.id))
            .order_by(ScreeningTask::ID)
            .get(self.t.db())
            .await
            .unwrap()
    }

    async fn messages(&self) -> Vec<Message> {
        Message::query()
            .order_by(Message::ID)
            .get(self.t.db())
            .await
            .unwrap()
    }
}

/// AC2: the MLRO and the deputy are told at once, with the task due 24 hours
/// later in Asia/Dubai; AC7: the alerts carry the party code and the due time
/// and nothing else; nothing goes to the party.
#[renox::test]
async fn a_flag_tells_the_mlro_and_the_deputy_at_once() {
    let w = world().await;
    w.screen(vec![candidate("Omar K.", 0.91)]).await;

    let tasks = w.tasks().await;
    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert_eq!(
        wl_parties::screening_task::dubai_label(task.flagged_at),
        "2026-11-03 14:00"
    );
    assert_eq!(
        wl_parties::screening_task::dubai_label(task.due_at),
        "2026-11-04 14:00"
    );
    assert_eq!(
        task.due_at,
        task.flagged_at + renox::chrono::Duration::hours(24)
    );

    for holder in [&w.mlro, &w.deputy] {
        let bells = holder.unread_notifications(w.t.db()).await.unwrap();
        assert_eq!(bells.len(), 1, "{}", holder.name);
        let message = bells[0].message().expect("a message");
        assert_eq!(message.title, "A possible sanctions match needs review");
        let body = message.body.clone().unwrap_or_default();
        assert!(body.contains(&w.party.code), "{body}");
        assert!(body.contains("2026-11-04 14:00"), "{body}");
        assert!(!body.contains("Omar") && !body.contains("UN consolidated"));
    }
    for other in [&w.partner] {
        assert!(
            other
                .unread_notifications(w.t.db())
                .await
                .unwrap()
                .is_empty()
        );
    }

    // One queued e-mail each, to the two staff addresses and no one else.
    let queued = w.messages().await;
    assert_eq!(queued.len(), 2);
    let mut users: Vec<Option<i64>> = queued.iter().map(|m| m.user_id).collect();
    users.sort();
    assert_eq!(users, vec![Some(w.mlro.id), Some(w.deputy.id)]);
    assert!(queued.iter().all(|m| m.type_key == POSSIBLE_MATCH_TYPE));
    // Nothing is queued to the party (no tipping off).
    assert!(queued.iter().all(|m| m.party_id.is_none()));

    let sent = w.mailer.0.lock().unwrap().clone();
    assert_eq!(sent.len(), 2);
    let mut addresses: Vec<&str> = sent.iter().map(|o| o.address.as_str()).collect();
    addresses.sort();
    assert_eq!(addresses, vec!["dina@example.com", "mariam@example.com"]);
    for mail in &sent {
        let text = format!("{} {}", mail.title, mail.body);
        assert!(text.contains(&w.party.code), "{text}");
        assert!(text.contains("2026-11-04 14:00"), "{text}");
        for hidden in ["Omar", "UN consolidated", "QDi.001", "0.91"] {
            assert!(!text.contains(hidden), "the mail shows {hidden}: {text}");
        }
        // "sanctions" only in the fixed title.
        assert!(
            !mail.body.to_lowercase().contains("sanctions"),
            "{}",
            mail.body
        );
    }

    // The task and its candidate show in each holder's list.
    let detail = format!("/parties/screening-tasks/{}", task.id);
    for holder in [&w.mlro, &w.deputy] {
        w.t.app.acting_as(holder);
        w.t.app
            .get("/parties/screening-tasks")
            .await
            .assert_ok()
            .assert_see(&detail)
            .assert_see(&w.party.code)
            .assert_see("2026-11-04 14:00");
        w.t.app
            .get(&detail)
            .await
            .assert_ok()
            .assert_see("Omar K.")
            .assert_see("0.91");
    }
    // The party is never blocked by the flag.
    assert_eq!(w.t.reload(&w.party).await.status, PartyStatus::Active);
}

/// AC3: a partner with `parties.view` only sees no task, count, badge, due
/// date or nav link, and the party stays as it was.
#[renox::test]
async fn a_partner_without_the_permission_sees_nothing() {
    let w = world().await;
    let page = format!("/parties/{}", w.party.id);
    w.t.app.acting_as(&w.partner);
    let before = (
        w.t.app.get(&page).await.assert_ok().text(),
        w.t.app.get("/parties").await.assert_ok().text(),
        w.t.app.get("/parties?export=csv").await.assert_ok().text(),
    );
    assert!(!before.1.contains("/parties/screening-tasks"));

    w.screen(vec![candidate("Omar K.", 0.91)]).await;
    let task = w.tasks().await.remove(0);

    w.t.app.acting_as(&w.partner);
    w.t.app
        .get("/parties/screening-tasks")
        .await
        .assert_not_found();
    w.t.app
        .get(&format!("/parties/screening-tasks/{}", task.id))
        .await
        .assert_not_found();
    let after = (
        w.t.app.get(&page).await.assert_ok().text(),
        w.t.app.get("/parties").await.assert_ok().text(),
        w.t.app.get("/parties?export=csv").await.assert_ok().text(),
    );
    assert_eq!(before, after, "the pages are unchanged by the flag");
    for text in [&after.0, &after.1, &after.2] {
        for hidden in [
            "screening-tasks",
            "2026-11-04",
            "Omar",
            "possible match",
            "UN consolidated",
        ] {
            assert!(!text.contains(hidden), "a partner sees {hidden}");
        }
    }
    assert!(after.0.contains("Active"), "{}", after.0);
    assert_eq!(w.t.reload(&w.party).await.status, PartyStatus::Active);

    // The compliance role has the nav link.
    w.t.app.acting_as(&w.mlro);
    w.t.app
        .get("/parties")
        .await
        .assert_ok()
        .assert_see("href=\"/parties/screening-tasks\"");
}

/// AC4: a re-screen at 18:00 the same day links to the open task, alerts no
/// one again and keeps the due time. AC5: a run with two candidates at or
/// above the threshold gives one task listing both.
#[renox::test]
async fn a_rescreen_is_linked_to_the_open_task_and_alerts_no_one() {
    let w = world().await;
    w.screen(vec![
        candidate("Omar K.", 0.91),
        candidate("Omar Khalil", 0.88),
        candidate("Omri Kahn", 0.40),
    ])
    .await;
    let first = w.tasks().await;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].candidates(w.t.db()).await.unwrap().len(), 3);

    set_clock(&w.t, 18);
    w.screen(vec![candidate("Omar K.", 0.95)]).await;

    let tasks = w.tasks().await;
    assert_eq!(tasks.len(), 1, "no second task");
    assert_eq!(tasks[0].due_at, first[0].due_at);
    assert_eq!(
        wl_parties::screening_task::dubai_label(tasks[0].due_at),
        "2026-11-04 14:00"
    );
    let screenings = tasks[0].screenings(w.t.db()).await.unwrap();
    assert_eq!(screenings.len(), 2);
    assert!(screenings.iter().all(|s| s.task_id == Some(tasks[0].id)));
    assert_eq!(tasks[0].candidates(w.t.db()).await.unwrap().len(), 4);

    // No second alert: one bell and one e-mail each, as after the flag.
    for holder in [&w.mlro, &w.deputy] {
        assert_eq!(
            holder.unread_notifications(w.t.db()).await.unwrap().len(),
            1
        );
    }
    assert_eq!(w.messages().await.len(), 2);
    assert_eq!(w.mailer.0.lock().unwrap().len(), 2);

    // The task page lists the candidates of both runs.
    w.t.app.acting_as(&w.mlro);
    w.t.app
        .get(&format!("/parties/screening-tasks/{}", tasks[0].id))
        .await
        .assert_ok()
        .assert_see("Omar K.")
        .assert_see("Omar Khalil")
        .assert_see("0.95")
        .assert_see("0.88")
        .assert_see("2026-11-04 14:00");
}
