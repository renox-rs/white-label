//! Alerting compliance and re-running screenings when a sanctions list is
//! stale (wl-parties §3, `PTY.RUL.02`): the acceptance checks AC1–AC10 of
//! #40, end to end, with a fake feed, the clock moved to fixed days in
//! Asia/Dubai, the MLRO, the deputy and a partner user, and a fake mailer.
//! The pieces are covered one by one in `freshness.rs`, `rescreen.rs`,
//! `sanctions_lists.rs` and `screening_list_page.rs`.

mod support;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use renox::chrono::{TimeZone, Utc};
use renox::prelude::*;
use wl_kernel::notify::NotifyChannel;
use wl_kernel::{RuleRun, RuleRunStatus};
use wl_notify::channels::ProviderFuture;
use wl_notify::{ChannelProvider, Outgoing, SendError, WlNotify};
use wl_parties::feed::{FeedError, FetchFuture};
use wl_parties::freshness::{self, RULE};
use wl_parties::{
    ListFeed, ListStatus, ListVersion, Party, SanctionsList, SanctionsListStale,
    SanctionsListUpdated, Screening, ScreeningTrigger, WlParties, permissions as perm, screening,
    seed,
};

use support::{Booted, boot, boot_custom};

const AL_NOOR: &str = "Al Noor Trading LLC";
const UN: &str = "UN Consolidated List";
const UAE: &str = "UAE Local Terrorist List";

/// A feed that answers per list what the test sets; a list it knows nothing
/// about is unreachable.
#[derive(Clone, Default)]
struct FakeFeed(Arc<Mutex<HashMap<String, String>>>);

impl FakeFeed {
    fn serve_all(&self, version: &str) {
        self.serve(UN, version);
        self.serve(UAE, version);
    }

    fn serve(&self, list: &str, version: &str) {
        self.0
            .lock()
            .unwrap()
            .insert(list.to_owned(), version.to_owned());
    }

    fn fail_all(&self) {
        self.0.lock().unwrap().clear();
    }
}

impl ListFeed for FakeFeed {
    fn fetch<'a>(&'a self, list: &'a SanctionsList) -> FetchFuture<'a> {
        let answer = self.0.lock().unwrap().get(&list.name).cloned();
        Box::pin(async move {
            answer
                .map(|version| ListVersion {
                    version,
                    published_at: None,
                })
                .ok_or_else(|| FeedError("source unreachable".to_owned()))
        })
    }
}

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

/// The app with screening, a fake feed, wl-notify and a fake mailer, the
/// MLRO, the deputy and a partner with `parties.view` only.
struct World {
    t: Booted,
    feed: FakeFeed,
    mailer: Mailer,
    stale: Arc<Mutex<Vec<SanctionsListStale>>>,
    updated: Arc<Mutex<Vec<SanctionsListUpdated>>>,
    party: Party,
    mlro: User,
    deputy: User,
    partner: User,
}

async fn world() -> World {
    let feed = FakeFeed::default();
    let mailer = Mailer::default();
    let stale = Arc::new(Mutex::new(Vec::new()));
    let updated = Arc::new(Mutex::new(Vec::new()));
    let (provider, s, u) = (mailer.clone(), Arc::clone(&stale), Arc::clone(&updated));
    let module = WlParties::new().screening(true).list_feed(feed.clone());
    let t = boot_custom(module, true, move |app| {
        app.module(WlNotify::new().provider(provider))
            .listen(move |event: SanctionsListStale, _state: AppState| {
                let s = Arc::clone(&s);
                async move {
                    s.lock().unwrap().push(event);
                    Ok(())
                }
            })
            .listen(move |event: SanctionsListUpdated, _state: AppState| {
                let u = Arc::clone(&u);
                async move {
                    u.lock().unwrap().push(event);
                    Ok(())
                }
            })
    })
    .await;
    t.app
        .kernel()
        .call("notify:install", Vec::<String>::new())
        .await
        .expect("notify:install runs");
    seed::lists(t.app.state()).await.unwrap();
    let party = t.party(AL_NOOR).await;
    let mlro = t.mlro().await;
    let deputy = t.user("Dina", &[perm::VIEW, perm::BLOCK_REASON_VIEW]).await;
    let partner = t.user("Pia", &[perm::VIEW, perm::EXPORT]).await;
    World {
        t,
        feed,
        mailer,
        stale,
        updated,
        party,
        mlro,
        deputy,
        partner,
    }
}

/// Moves the clock to `day` November 2026, `hour`:00:10 Asia/Dubai (UTC+4).
fn set_clock(t: &Booted, day: u32, dubai_hour: u32) {
    t.app.travel_back();
    let wanted = Utc
        .with_ymd_and_hms(2026, 11, day, dubai_hour - 4, 0, 10)
        .unwrap();
    let ahead = (wanted - Utc::now()).num_seconds();
    assert!(ahead > 0, "the fixed day is in the future");
    t.app.travel(Duration::from_secs(ahead.unsigned_abs()));
}

impl World {
    fn hours(&self, n: u64) {
        self.t.app.travel(Duration::from_secs(n * 3600));
    }

    /// The hourly task now, then the queue worked off.
    async fn check(&self) -> freshness::Report {
        let report = self
            .t
            .app
            .at_travelled_time(freshness::run(self.t.app.state()))
            .await
            .unwrap();
        self.t.app.run_all_jobs().await;
        report
    }

    async fn list(&self, name: &str) -> SanctionsList {
        SanctionsList::all(self.t.db())
            .await
            .unwrap()
            .into_iter()
            .find(|l| l.name == name)
            .unwrap()
    }

    async fn screenings(&self) -> Vec<Screening> {
        let mut rows = Screening::for_party(self.t.db(), self.party.id)
            .await
            .unwrap();
        rows.sort_by_key(|s| s.id);
        rows
    }

    /// The bell entries of `user` about `list`.
    async fn bells(&self, user: &User, list: &str) -> usize {
        user.unread_notifications(self.t.db())
            .await
            .unwrap()
            .iter()
            .filter(|n| {
                n.message()
                    .is_some_and(|m| m.body.clone().unwrap_or_default().contains(list))
            })
            .count()
    }

    /// The e-mails sent to `address` about `list`.
    fn mails(&self, address: &str, list: &str) -> Vec<Outgoing> {
        self.mailer
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.address == address && m.body.contains(list))
            .cloned()
            .collect()
    }

    fn sent(&self) -> usize {
        self.mailer.0.lock().unwrap().len()
    }
}

/// AC1: 24 hours without a successful download turn the list stale; the
/// MLRO and the deputy get one bell notice and one e-mail per list, a user
/// without the permission gets none. AC2: hourly retries and a later failure
/// in the same period alert nobody again. (The exact 24 hour boundary is the
/// pure `is_stale` check, unit tested in `src/sanctions_lists.rs`.)
#[renox::test]
async fn a_list_that_turns_stale_alerts_compliance_once() {
    let w = world().await;
    set_clock(&w.t, 2, 6);
    w.feed.serve_all("v1");
    w.check().await;
    assert_eq!(w.sent(), 0, "a current list alerts nobody");
    let list = w.list(UN).await;
    let last_update = list.last_updated_at.unwrap();

    // 2026-11-03 06:00: 24 hours and the download fails.
    w.feed.fail_all();
    w.hours(24);
    let report = w.check().await;
    assert_eq!(report.stale, 2);
    let list = w.list(UN).await;
    assert_eq!(list.status, ListStatus::Stale);
    assert!(list.stale_since.is_some());
    assert_eq!(list.last_updated_at, Some(last_update));
    let events = w.stale.lock().unwrap().clone();
    assert_eq!(events.iter().filter(|e| e.name == UN).count(), 1);
    assert_eq!(events.len(), 2, "one per list");

    for (holder, address) in [
        (&w.mlro, "mariam@example.com"),
        (&w.deputy, "dina@example.com"),
    ] {
        assert_eq!(w.bells(holder, UN).await, 1, "{}", holder.name);
        let mails = w.mails(address, UN);
        assert_eq!(mails.len(), 1, "{address}");
        let text = format!("{} {}", mails[0].title, mails[0].body);
        assert!(text.contains("2026-11-02 06:00"), "{text}");
        assert!(text.contains("source unreachable"), "{text}");
    }
    assert!(
        w.partner
            .unread_notifications(w.t.db())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        w.mailer
            .0
            .lock()
            .unwrap()
            .iter()
            .all(|m| m.address != "pia@example.com")
    );
    let sent = w.sent();
    assert_eq!(sent, 4, "two holders, two lists");

    // AC2: retried at 07:00 and 08:00, a failure at 09:00, retried at 10:00
    // and 11:00; no second event, bell notice or e-mail.
    let mut attempt = w.list(UN).await.last_attempt_at.unwrap();
    for _ in 0..5 {
        w.hours(1);
        let report = w.check().await;
        assert_eq!((report.failed_downloads, report.stale), (2, 0));
        let list = w.list(UN).await;
        assert_eq!(list.status, ListStatus::Stale);
        assert_eq!(list.last_error.as_deref(), Some("source unreachable"));
        assert!(list.last_attempt_at.unwrap() > attempt, "retried");
        attempt = list.last_attempt_at.unwrap();
        assert_eq!(w.stale.lock().unwrap().len(), 2);
        assert_eq!(w.bells(&w.mlro, UN).await, 1);
        assert_eq!(w.sent(), sent);
    }
}

/// AC3 (first half): before 24 hours with a working feed returning the held
/// version the list stays current and nothing is sent. AC6: the version
/// already held refreshes the last update, clears stale, emits no update and
/// starts no re-run.
#[renox::test]
async fn the_held_version_refreshes_the_list_without_an_update() {
    let w = world().await;
    set_clock(&w.t, 2, 6);
    w.feed.serve_all("v1");
    w.check().await;
    assert_eq!(w.updated.lock().unwrap().len(), 2);

    // 05:00 on the next day: still current, same version.
    w.hours(23);
    let report = w.check().await;
    assert_eq!((report.updated, report.stale), (0, 0));
    let list = w.list(UN).await;
    assert_eq!(list.status, ListStatus::Current);
    assert_eq!(w.sent(), 0);
    assert!(w.stale.lock().unwrap().is_empty());

    // Stale (feed down for a day), then the same version again.
    w.feed.fail_all();
    w.hours(24);
    w.check().await;
    assert_eq!(w.list(UN).await.status, ListStatus::Stale);
    let run = screening::run(w.t.app.state(), &w.party, ScreeningTrigger::Manual)
        .await
        .unwrap();
    assert!(run.lists_stale);
    let before = w.list(UN).await;
    w.feed.serve_all("v1");
    w.hours(1);
    let report = w.check().await;
    assert_eq!((report.updated, report.stale), (0, 0));
    let after = w.list(UN).await;
    assert_eq!(after.status, ListStatus::Current);
    assert!(after.stale_since.is_none() && after.last_error.is_none());
    assert!(after.last_updated_at > before.last_updated_at);
    assert_eq!(after.version.as_deref(), Some("v1"));
    assert_eq!(w.updated.lock().unwrap().len(), 2, "no new update event");
    assert_eq!(w.screenings().await.len(), 1, "nothing re-run");
}

/// AC4–AC7: a screening run on a stale list is flagged and shows the lists;
/// the next version clears the list and re-runs it as a new screening; a new
/// failure after that is a new stale period with a new alert.
#[renox::test]
async fn a_stale_screening_is_flagged_then_rerun_and_a_new_period_alerts_again() {
    let w = world().await;
    set_clock(&w.t, 2, 6);
    w.feed.serve_all("v1");
    w.check().await;
    w.feed.fail_all();
    w.hours(24);
    w.check().await;
    assert_eq!(w.stale.lock().unwrap().len(), 2);

    // AC4: 2026-11-03 12:00.
    w.hours(6);
    w.check().await;
    let flagged = screening::run(w.t.app.state(), &w.party, ScreeningTrigger::Manual)
        .await
        .unwrap();
    assert!(flagged.lists_stale);
    let used = flagged.lists_used.0.iter().find(|l| l.name == UN).unwrap();
    assert_eq!(used.version.as_deref(), Some("v1"));
    assert!(used.stale && used.last_updated_at.is_some());
    for user in [&w.mlro, &w.deputy] {
        w.t.app.acting_as(user);
        w.t.app
            .get("/parties/screening")
            .await
            .assert_ok()
            .assert_see(UN)
            .assert_see("2026-11-02 06:00")
            .assert_see("Stale list")
            .assert_see(&w.party.code);
    }

    // AC5: 15:00, a new version.
    w.hours(3);
    w.feed.serve_all("v2");
    let report = w.check().await;
    assert_eq!((report.updated, report.stale), (2, 0));
    for name in [UN, UAE] {
        let list = w.list(name).await;
        assert_eq!(list.status, ListStatus::Current);
        assert_eq!(list.version.as_deref(), Some("v2"));
        assert!(list.stale_since.is_none() && list.last_error.is_none());
        let events = w.updated.lock().unwrap().clone();
        let own = events
            .iter()
            .filter(|e| e.name == name && e.version == "v2");
        assert_eq!(own.count(), 1, "{name}");
    }
    let rows = w.screenings().await;
    let last = rows.last().unwrap();
    assert_ne!(last.id, flagged.id);
    assert_eq!(last.run_trigger, ScreeningTrigger::ListUpdate);
    assert!(!last.lists_stale);
    assert!(
        last.lists_used
            .0
            .iter()
            .all(|l| !l.stale && l.version.as_deref() == Some("v2"))
    );
    assert_eq!(rows[0].id, flagged.id);
    assert!(rows[0].superseded_by.is_some());
    assert_eq!(last.superseded_by, None);

    // A second update re-runs nothing for it: the same version later, and a
    // newer one with nothing flagged waiting.
    let count = rows.len();
    w.hours(12);
    w.check().await;
    assert_eq!(w.screenings().await.len(), count);
    w.feed.serve_all("v3");
    w.hours(12);
    w.check().await;
    assert_eq!(w.screenings().await.len(), count);

    // AC7: the feed fails again; a day later a new period, a new alert.
    w.feed.fail_all();
    w.hours(24);
    let report = w.check().await;
    assert_eq!(report.stale, 2);
    assert_eq!(w.stale.lock().unwrap().len(), 4);
    assert_eq!(w.bells(&w.mlro, UN).await, 2);
    assert_eq!(w.mails("dina@example.com", UN).len(), 2);
}

/// AC8: with a second list still stale, the re-run for the first list's
/// update is flagged again and is re-run once that list updates.
#[renox::test]
async fn a_rerun_with_another_stale_list_is_flagged_and_rerun_again() {
    let w = world().await;
    set_clock(&w.t, 3, 12);
    let first = screening::run(w.t.app.state(), &w.party, ScreeningTrigger::Onboarding)
        .await
        .unwrap();
    assert!(first.lists_stale);

    w.feed.serve(UN, "v1");
    w.check().await;
    assert_eq!(w.list(UN).await.status, ListStatus::Current);
    assert_eq!(w.list(UAE).await.status, ListStatus::Stale);
    let rows = w.screenings().await;
    assert_eq!(rows.len(), 2);
    assert!(rows[1].lists_stale, "the UAE list is still stale");
    assert_eq!(rows[1].rerun_of, Some(first.id));
    assert_eq!(rows[0].superseded_by, Some(rows[1].id));

    w.hours(1);
    w.feed.serve(UAE, "u1");
    w.check().await;
    let rows = w.screenings().await;
    assert_eq!(rows.len(), 3);
    assert!(!rows[2].lists_stale);
    assert_eq!(rows[2].rerun_of, Some(rows[1].id));
    assert_eq!(rows[1].superseded_by, Some(rows[2].id));
    assert_eq!(rows[2].run_trigger, ScreeningTrigger::ListUpdate);
}

/// AC9: one `PTY.RUL.02` run per list and hour; with screening off the
/// command does nothing and creates no run.
#[renox::test]
async fn the_check_is_one_rule_run_per_list_and_hour() {
    let w = world().await;
    set_clock(&w.t, 2, 6);
    w.feed.serve_all("v1");
    let first = w.check().await;
    assert_eq!((first.checked, first.skipped), (2, 0));
    let again = w.check().await;
    assert_eq!((again.skipped, again.updated), (2, 0));
    let runs = RuleRun::query().get(w.t.db()).await.unwrap();
    assert_eq!(runs.len(), 2);
    assert!(
        runs.iter()
            .all(|r| r.rule == RULE && r.rule == "PTY.RUL.02")
    );
    assert!(runs.iter().all(|r| r.status == RuleRunStatus::Succeeded));
    let mut subjects: Vec<i64> = runs.iter().map(|r| r.subject_id).collect();
    subjects.sort_unstable();
    let mut lists: Vec<i64> = SanctionsList::all(w.t.db())
        .await
        .unwrap()
        .iter()
        .map(|l| l.id)
        .collect();
    lists.sort_unstable();
    assert_eq!(subjects, lists);
    w.hours(1);
    w.check().await;
    assert_eq!(RuleRun::query().get(w.t.db()).await.unwrap().len(), 4);

    let off = boot().await;
    off.app
        .kernel()
        .call(freshness::TASK, Vec::<String>::new())
        .await
        .expect("the command runs");
    assert!(RuleRun::query().get(off.db()).await.unwrap().is_empty());
}

/// AC10: the page shows version and last update per list to the three
/// permissions, is forbidden to anyone else, and no list data reaches a
/// user without them.
#[renox::test]
async fn only_compliance_sees_the_lists() {
    let w = world().await;
    set_clock(&w.t, 2, 6);
    w.feed.serve_all("v7");
    w.check().await;
    screening::run(w.t.app.state(), &w.party, ScreeningTrigger::Manual)
        .await
        .unwrap();

    for (name, grant) in [
        ("Rana", perm::SCREENING_RUN),
        ("Rev", perm::SCREENING_REVIEW),
        ("Mona", perm::BLOCK_REASON_VIEW),
    ] {
        let user = w.t.user(name, &[grant]).await;
        w.t.app.acting_as(&user);
        w.t.app
            .get("/parties/screening")
            .await
            .assert_ok()
            .assert_see(UN)
            .assert_see(UAE)
            .assert_see("v7")
            .assert_see("2026-11-02 06:00");
    }

    w.t.app.acting_as(&w.partner);
    w.t.app.get("/parties/screening").await.assert_forbidden();
    w.t.app
        .get(&format!("/parties/{}", w.party.id))
        .await
        .assert_ok()
        .assert_dont_see(UN)
        .assert_dont_see("v7")
        .assert_dont_see("Stale list");
    w.t.app
        .get("/parties")
        .await
        .assert_ok()
        .assert_dont_see(UN);
}
