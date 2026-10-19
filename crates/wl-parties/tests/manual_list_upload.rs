//! Uploading an official sanctions list by hand (wl-parties §5.4, issue #42
//! part 5): the acceptance checks that need the whole app, with the real
//! listeners (the re-run after an update), a fake feed, the MLRO, a
//! block-reason-only user, a partner and a fixed clock. The pieces are
//! covered one by one in `list_upload_model.rs`, `list_upload_service.rs`,
//! `manual_check_service.rs` and `list_upload_pages.rs`.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use renox::prelude::*;
use wl_kernel::{PartyStatus, ScreeningStatus};
use wl_notify::WlNotify;
use wl_parties::feed::{FeedError, FetchFuture};
use wl_parties::screening::{self, CheckFuture};
use wl_parties::{
    ListFeed, ListFormat, ListStatus, ListUsed, ListVersion, NewCandidate, Party, ProviderError,
    SanctionsList, SanctionsListUpdated, SanctionsListUpload, Screening, ScreeningMethod,
    ScreeningTask, ScreeningTrigger, UpdateSource, WlParties, freshness, permissions as perm, seed,
};

use support::{Booted, archive_party, boot_custom, set_dubai_clock};

const UAE: &str = "UAE Local Terrorist List";
const PDF: &str = "Local list (PDF) of 2026-11-03";
const AL_NOOR: &str = "Al Noor Trading LLC";
const XML: &[u8] = b"<list><entry><name>Omar K.</name></entry></list>";
const URL: &str = "https://example.org/uae-list";

#[derive(Clone, Default)]
struct FakeFeed(Arc<Mutex<Option<String>>>);

impl ListFeed for FakeFeed {
    fn fetch<'a>(&'a self, _list: &'a SanctionsList) -> FetchFuture<'a> {
        let answer = self.0.lock().unwrap().clone();
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

/// A provider that finds nothing.
struct NoMatches;

impl wl_parties::ScreeningProvider for NoMatches {
    fn check<'a>(&'a self, _party: &'a Party, _lists: &'a [ListUsed]) -> CheckFuture<'a> {
        Box::pin(async { Ok::<Vec<NewCandidate>, ProviderError>(Vec::new()) })
    }
}

struct World {
    t: Booted,
    feed: FakeFeed,
    updated: Arc<Mutex<Vec<SanctionsListUpdated>>>,
}

async fn world() -> World {
    let feed = FakeFeed::default();
    let updated = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&updated);
    let module = WlParties::new()
        .screening(true)
        .list_feed(feed.clone())
        .screening_provider(NoMatches);
    let t = boot_custom(module, true, move |app| {
        app.module(WlNotify::new())
            .listen(move |event: SanctionsListUpdated, _state: AppState| {
                let sink = Arc::clone(&sink);
                async move {
                    sink.lock().unwrap().push(event);
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
    // Today is 2026-11-05 in Dubai.
    set_dubai_clock(&t, 5, 10);
    World { t, feed, updated }
}

impl World {
    async fn list(&self, name: &str) -> SanctionsList {
        SanctionsList::all(self.t.db())
            .await
            .unwrap()
            .into_iter()
            .find(|l| l.name == name)
            .unwrap()
    }

    async fn mlro(&self) -> User {
        self.t
            .user(
                "Mariam",
                &[perm::VIEW, perm::SCREENING_REVIEW, perm::BLOCK_REASON_VIEW],
            )
            .await
    }

    async fn upload(&self, list: &SanctionsList, date: &str) -> renox::testing::TestResponse {
        self.t
            .app
            .htmx()
            .post_multipart(
                &format!("/parties/screening/lists/{}/upload", list.id),
                &[("source_url", URL), ("list_date", date)],
                &[("file", "uae.xml", XML)],
            )
            .await
    }

    async fn screenings(&self, party: &Party) -> Vec<Screening> {
        let mut rows = Screening::for_party(self.t.db(), party.id).await.unwrap();
        rows.sort_by_key(|s| s.id);
        rows
    }

    async fn pdf_list(&self) -> SanctionsList {
        let mut list = SanctionsList {
            name: PDF.into(),
            source: "https://example.org".into(),
            format: ListFormat::Pdf,
            status: ListStatus::Current,
            version: Some("2026-11-03".into()),
            ..SanctionsList::default()
        };
        list.insert(self.t.db()).await.unwrap();
        list
    }

    async fn manual_check(
        &self,
        party: &Party,
        list: &SanctionsList,
        fields: &[(&str, &str)],
    ) -> renox::testing::TestResponse {
        let id = list.id.to_string();
        let mut all = vec![("list_id", id.as_str()), ("list_date", "2026-11-03")];
        all.extend_from_slice(fields);
        self.t
            .app
            .htmx()
            .post(&format!("/parties/{}/manual-check", party.id), &all)
            .await
    }
}

/// AC1: the upload makes the list current, emits the event once and the
/// flagged Screening is re-run as a new one with trigger `list_update`.
#[renox::test]
async fn an_upload_updates_the_list_and_reruns_the_flagged_screening() {
    let w = world().await;
    let party = w.t.party(AL_NOOR).await;
    let first = screening::run(w.t.app.state(), &party, ScreeningTrigger::Onboarding)
        .await
        .unwrap();
    assert!(first.lists_stale);

    let mlro = w.mlro().await;
    w.t.app.acting_as(&mlro);
    let uae = w.list(UAE).await;
    assert_eq!(uae.status, ListStatus::Stale);
    w.upload(&uae, "2026-11-03").await.assert_ok();
    w.t.app.run_all_jobs().await;

    let after = w.list(UAE).await;
    assert_eq!(after.status, ListStatus::Current);
    assert_eq!(after.update_source, Some(UpdateSource::Manual));
    assert_eq!(after.updated_by, Some(mlro.id));
    assert_eq!(after.source_url.as_deref(), Some(URL));
    assert!(after.stale_since.is_none() && after.last_error.is_none());
    assert_eq!(w.updated.lock().unwrap().len(), 1);

    let rows = w.screenings(&party).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].run_trigger, ScreeningTrigger::ListUpdate);
    assert_eq!(rows[1].rerun_of, Some(rows[0].id));
    assert_eq!(rows[0].superseded_by, Some(rows[1].id));
    let used = rows[1]
        .lists_used
        .0
        .iter()
        .find(|l| l.list_id == uae.id)
        .unwrap();
    assert_eq!(used.version.as_deref(), Some("2026-11-03"));
}

/// AC2 and AC3 with the real listeners: a refused upload and the same date
/// change nothing downstream (no event, no re-run).
#[renox::test]
async fn refused_and_same_date_uploads_start_no_rerun() {
    let w = world().await;
    let party = w.t.party(AL_NOOR).await;
    screening::run(w.t.app.state(), &party, ScreeningTrigger::Onboarding)
        .await
        .unwrap();
    let mlro = w.mlro().await;
    w.t.app.acting_as(&mlro);
    let uae = w.list(UAE).await;

    // Not well-formed: refused, still stale, logged as refused.
    w.t.app
        .htmx()
        .post_multipart(
            &format!("/parties/screening/lists/{}/upload", uae.id),
            &[("source_url", URL), ("list_date", "2026-11-03")],
            &[("file", "uae.xml", b"<list><entry>".as_slice())],
        )
        .await
        .assert_status(422);
    let log = SanctionsListUpload::for_list(w.t.db(), uae.id)
        .await
        .unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(w.list(UAE).await.status, ListStatus::Stale);

    // The same date as the current version: accepted, nothing re-run.
    let mut same = w.list(UAE).await;
    same.version = Some("2026-11-03".into());
    same.save(w.t.db()).await.unwrap();
    w.upload(&uae, "2026-11-03").await.assert_ok();
    w.t.app.run_all_jobs().await;
    assert_eq!(w.list(UAE).await.status, ListStatus::Current);
    assert!(w.updated.lock().unwrap().is_empty());
    assert_eq!(w.screenings(&party).await.len(), 1);
}

/// AC5: a partner and a block-reason-only user get 403 on both routes and
/// see neither button; the MLRO sees both.
#[renox::test]
async fn only_screening_review_reaches_the_upload_and_the_manual_check() {
    let w = world().await;
    let pdf = w.pdf_list().await;
    let uae = w.list(UAE).await;
    let party = w.t.party(AL_NOOR).await;
    let page = format!("/parties/{}", party.id);

    for (name, grants) in [
        ("Pia", vec![perm::VIEW, perm::EXPORT]),
        ("Dina", vec![perm::VIEW, perm::BLOCK_REASON_VIEW]),
    ] {
        let user = w.t.user(name, &grants).await;
        w.t.app.acting_as(&user);
        w.t.app
            .get("/parties/screening")
            .await
            .assert_dont_see("Upload official file");
        w.t.app
            .get(&page)
            .await
            .assert_ok()
            .assert_dont_see("Record manual check");
        w.upload(&uae, "2026-11-03").await.assert_status(403);
        w.manual_check(&party, &pdf, &[("result", "no_match")])
            .await
            .assert_status(403);
    }
    assert_eq!(w.list(UAE).await.status, ListStatus::Stale);
    assert!(w.screenings(&party).await.is_empty());

    let mlro = w.mlro().await;
    w.t.app.acting_as(&mlro);
    w.t.app
        .get("/parties/screening")
        .await
        .assert_ok()
        .assert_see("Upload official file");
    w.t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see("Record manual check");
}

/// AC6: a later download with a newer version replaces the manual one.
#[renox::test]
async fn a_newer_download_replaces_the_manual_upload() {
    let w = world().await;
    let mlro = w.mlro().await;
    w.t.app.acting_as(&mlro);
    let uae = w.list(UAE).await;
    w.upload(&uae, "2026-11-03").await.assert_ok();
    assert_eq!(w.list(UAE).await.updated_by, Some(mlro.id));

    w.feed.0.lock().unwrap().replace("2026-11-10".to_owned());
    w.t.app.travel(Duration::from_secs(25 * 3600));
    w.t.app
        .at_travelled_time(freshness::run(w.t.app.state()))
        .await
        .unwrap();

    let after = w.list(UAE).await;
    assert_eq!(after.version.as_deref(), Some("2026-11-10"));
    assert_eq!(after.update_source, Some(UpdateSource::Download));
    assert_eq!(after.updated_by, None);
    assert_eq!(after.source_url, None);
    assert_eq!(after.status, ListStatus::Current);
}

/// AC7 and AC8 end to end: "no match" is clear; a candidate with a list
/// entry is a possible match with one task due 24 hours later and the
/// compliance alert, and the party is not blocked.
#[renox::test]
async fn a_manual_check_clears_or_flags_without_blocking() {
    let w = world().await;
    let pdf = w.pdf_list().await;
    let mlro = w.mlro().await;
    w.t.app.acting_as(&mlro);
    let omar = w.t.party(AL_NOOR).await;

    w.manual_check(&omar, &pdf, &[("result", "no_match")])
        .await
        .assert_ok();
    let rows = w.screenings(&omar).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, ScreeningStatus::Clear);
    assert_eq!(rows[0].method, ScreeningMethod::Manual);
    assert_eq!(rows[0].run_trigger, ScreeningTrigger::Manual);
    assert_eq!(rows[0].reviewer_user, Some(mlro.id));
    assert_eq!(rows[0].lists_used.0[0].list_id, pdf.id);
    assert_eq!(
        rows[0].lists_used.0[0].version.as_deref(),
        Some("2026-11-03")
    );

    let candidate = [
        ("result", "candidate"),
        ("name", "Omar K."),
        ("list_entry", "UAE-12"),
    ];
    w.manual_check(&omar, &pdf, &candidate).await.assert_ok();
    w.t.app.run_all_jobs().await;
    let rows = w.screenings(&omar).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].status, ScreeningStatus::PossibleMatch);
    let tasks = ScreeningTask::open_tasks(w.t.db()).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].due_at,
        tasks[0].flagged_at + renox::chrono::Duration::hours(24)
    );
    let bells = mlro.unread_notifications(w.t.db()).await.unwrap();
    assert_eq!(bells.len(), 1, "the compliance alert");
    assert!(w.t.reload(&omar).await.status != PartyStatus::Blocked);

    // A second candidate links to the same task: no second task.
    w.manual_check(&omar, &pdf, &candidate).await.assert_ok();
    assert_eq!(ScreeningTask::open_tasks(w.t.db()).await.unwrap().len(), 1);
}

/// AC9 through the real route: every refusal stores nothing.
#[renox::test]
async fn manual_check_refusals_store_nothing() {
    let w = world().await;
    let pdf = w.pdf_list().await;
    let uae = w.list(UAE).await;
    let mlro = w.mlro().await;
    w.t.app.acting_as(&mlro);
    let party = w.t.party(AL_NOOR).await;

    // A non-PDF list.
    w.manual_check(&party, &uae, &[("result", "no_match")])
        .await
        .assert_status(422);
    // A candidate without a list entry.
    w.manual_check(&party, &pdf, &[("result", "candidate"), ("name", "Omar")])
        .await
        .assert_status(422);
    // A missing list date.
    w.t.app
        .htmx()
        .post(
            &format!("/parties/{}/manual-check", party.id),
            &[
                ("list_id", pdf.id.to_string().as_str()),
                ("list_date", ""),
                ("result", "no_match"),
            ],
        )
        .await
        .assert_status(422);
    // An archived party.
    archive_party(&w.t, &party).await;
    w.manual_check(&party, &pdf, &[("result", "no_match")])
        .await
        .assert_status(422);
    assert!(w.screenings(&party).await.is_empty());
    assert!(
        ScreeningTask::open_tasks(w.t.db())
            .await
            .unwrap()
            .is_empty()
    );
}
