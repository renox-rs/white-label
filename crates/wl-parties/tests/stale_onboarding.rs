//! Acceptance checks AC1-AC8 of #41 (BL-PTY-08): warn or block onboarding
//! while a sanctions list is stale.
//!
//! The engagement is a test-only model with an `activate` transition that a
//! vertical would run through `run_screened`. Its route acts as the signed-in
//! user, because a test cannot build an `AuthUser`. The lists go stale the
//! way they do in operation: a fake feed that stops answering and travelled
//! time. The unit-level guard is in `screening_guard.rs`, the party page
//! line for each permission in `stale_notice_page.rs`, and the kernel side
//! (the parties module off) in `crates/wl-kernel/tests/screening_guard.rs`.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use renox::db::Migration;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, history};
use serde::{Deserialize, Serialize};
use wl_kernel::screening_guard::StaleListPolicy;
use wl_kernel::screening_guard::{ScreenedError, ScreeningOutcome, StaleList, run_screened};
use wl_kernel::{PartyId, ScreeningStatus};
use wl_parties::feed::{FeedError, FetchFuture};
use wl_parties::screening::{self, CheckFuture};
use wl_parties::{
    ListFeed, ListStatus, ListUsed, ListVersion, NewCandidate, Party, ProviderError, SanctionsList,
    Screening, ScreeningProvider, ScreeningTrigger, WlParties, freshness, permissions as perm,
    seed,
};

use support::{Booted, boot_custom, set_dubai_clock};

const AL_NOOR: &str = "Al Noor Trading LLC";
const UN: &str = "UN Consolidated List";
const UAE: &str = "UAE Local Terrorist List";
const UN_DATED: &str = "UN Consolidated List of 2026-11-02";
const NOTICE: &str = "screened on a stale list (UN Consolidated List of 2026-11-02)";
const REFUSAL: &str = "Screening list is out of date: UN Consolidated List of 2026-11-02";

// ---------------------------------------------------------------------------
// The test-only engagement.
// ---------------------------------------------------------------------------

#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum EngagementStatus {
    #[default]
    Draft,
    Active,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_si_engagements", state_machine)]
struct Engagement {
    id: i64,
    status: EngagementStatus,
    party_id: i64,
}

impl StateMachine for Engagement {
    type State = EngagementStatus;
    const COLUMN: renox::db::Col<Engagement, EngagementStatus> = Engagement::STATUS;

    fn machine() -> Machine<Self> {
        use EngagementStatus::{Active, Draft};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .state(Active)
            .terminal()
            .transition(Transition::new("activate").from([Draft]).to(Active))
    }
}

const UP: &str = r#"
CREATE TABLE "test_si_engagements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    status TEXT NOT NULL DEFAULT 'draft',
    party_id INTEGER NOT NULL DEFAULT 0
);
"#;

const DOWN: &str = r#"DROP TABLE "test_si_engagements";"#;

const MIGRATIONS: &[Migration] = &[Migration::new(
    "20261019130004_create_test_stale_onboarding_engagements",
    UP,
    Some(DOWN),
)];

/// `activate` as the signed-in user: "active", or "warn: <lists>", or the
/// refusal as a 422.
async fn activate(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<String> {
    let record = Engagement::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let done = run_screened(
        &state,
        &user,
        &record,
        "activate",
        PartyId(record.party_id),
        json!({}),
    )
    .await
    .map_err(ScreenedError::into_error)?;
    Ok(match done.screening {
        ScreeningOutcome::Warn(lists) => format!(
            "warn: {}",
            lists
                .iter()
                .map(StaleList::label_en)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => "active".to_owned(),
    })
}

struct Engagements;

impl Module for Engagements {
    fn name(&self) -> &'static str {
        "test-stale-onboarding"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .post("/si/engagements/{id}/activate", activate)
            .require_auth()
    }

    fn migrations(&self) -> &'static [Migration] {
        MIGRATIONS
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Engagement>();
    }
}

// ---------------------------------------------------------------------------
// The fake feed and provider.
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct FakeFeed(Arc<Mutex<Option<String>>>);

impl FakeFeed {
    fn serve(&self, version: &str) {
        *self.0.lock().unwrap() = Some(version.to_owned());
    }

    fn fail(&self) {
        *self.0.lock().unwrap() = None;
    }
}

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

struct NoMatches;

impl ScreeningProvider for NoMatches {
    fn check<'a>(&'a self, _party: &'a Party, _lists: &'a [ListUsed]) -> CheckFuture<'a> {
        Box::pin(async { Ok::<Vec<NewCandidate>, ProviderError>(Vec::new()) })
    }
}

// ---------------------------------------------------------------------------
// The world.
// ---------------------------------------------------------------------------

struct World {
    t: Booted,
    feed: FakeFeed,
    party: Party,
    partner: User,
    compliance: User,
}

async fn world_with(module: WlParties, feed: FakeFeed) -> World {
    let t = boot_custom(module, true, |app| app.module(Engagements)).await;
    seed::lists(t.app.state()).await.unwrap();
    let party = t.party(AL_NOOR).await;
    let partner = t.user("Pia", &[perm::VIEW]).await;
    let compliance = t.user("Carla", &[perm::VIEW, perm::SCREENING_RUN]).await;
    World {
        t,
        feed,
        party,
        partner,
        compliance,
    }
}

async fn world(policy: StaleListPolicy) -> World {
    let feed = FakeFeed::default();
    let module = WlParties::new()
        .screening(true)
        .list_feed(feed.clone())
        .screening_provider(NoMatches)
        .stale_list_at_onboarding(policy);
    world_with(module, feed).await
}

impl World {
    /// Both lists download on 2026-11-02 (14:00 Dubai); then the feed stops
    /// answering and 25 hours pass, so both lists turn stale with the UN
    /// list last updated on 2026-11-02.
    async fn go_stale(&self) {
        set_dubai_clock(&self.t, 2, 14);
        self.feed.serve("v0");
        self.freshness().await;
        self.feed.fail();
        self.t.app.travel(Duration::from_secs(25 * 3600));
        self.freshness().await;
        for list in self.lists().await {
            assert_eq!(list.status, ListStatus::Stale, "{}", list.name);
        }
    }

    async fn freshness(&self) -> freshness::Report {
        self.t
            .app
            .at_travelled_time(freshness::run(self.t.app.state()))
            .await
            .unwrap()
    }

    async fn lists(&self) -> Vec<SanctionsList> {
        SanctionsList::all(self.t.db()).await.unwrap()
    }

    /// The clock as the app sees it.
    async fn now(&self) -> renox::db::DateTime {
        self.t
            .app
            .at_travelled_time(async { renox::db::now() })
            .await
    }

    /// Marks `names` as updated just now, without any screening re-run.
    async fn refresh(&self, names: &[&str]) {
        let now = self.now().await;
        for mut list in self.lists().await {
            if names.contains(&list.name.as_str()) {
                list.last_updated_at = Some(now);
                list.status = ListStatus::Current;
                list.stale_since = None;
                list.last_error = None;
                list.save(self.t.db()).await.unwrap();
            }
        }
    }

    /// An onboarding screening of the party, at the travelled time.
    async fn screen(&self) -> Screening {
        self.t
            .app
            .at_travelled_time(screening::run(
                self.t.app.state(),
                &self.party,
                ScreeningTrigger::Onboarding,
            ))
            .await
            .unwrap()
    }

    async fn screenings(&self) -> Vec<Screening> {
        let mut rows = Screening::for_party(self.t.db(), self.party.id)
            .await
            .unwrap();
        rows.sort_by_key(|s| s.id);
        rows
    }

    async fn engagement(&self) -> Engagement {
        Engagement::create(
            self.t.db(),
            Engagement {
                party_id: self.party.id,
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }

    /// `user` activates `engagement`.
    async fn activate(&self, user: &User, engagement: &Engagement) -> renox::testing::TestResponse {
        self.t.app.acting_as(user);
        self.t
            .app
            .post(
                &format!("/si/engagements/{}/activate", engagement.id),
                &[("x", "y")],
            )
            .await
    }

    async fn status(&self, engagement: &Engagement) -> EngagementStatus {
        Engagement::find(self.t.db(), engagement.id)
            .await
            .unwrap()
            .unwrap()
            .status
    }

    async fn history(&self, engagement: &Engagement) -> Vec<String> {
        history(self.t.db(), Engagement::TABLE, engagement.id)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.transition)
            .collect()
    }

    /// The party page as `user`.
    async fn page(&self, user: &User) -> renox::testing::TestResponse {
        self.t.app.acting_as(user);
        self.t.app.get(&format!("/parties/{}", self.party.id)).await
    }
}

// ---------------------------------------------------------------------------
// AC1-AC8.
// ---------------------------------------------------------------------------

/// AC1: with the default setting (`Warn`) and the UN list stale since
/// 2026-11-03, the activation succeeds, names the UN list and its last
/// update, and the party page shows the line.
#[renox::test]
async fn ac1_warn_activates_names_the_list_and_the_party_page_shows_the_line() {
    let w = world(StaleListPolicy::default()).await;
    assert_eq!(StaleListPolicy::default(), StaleListPolicy::Warn);
    w.go_stale().await;
    w.refresh(&[UAE]).await;
    let un = w.lists().await.into_iter().find(|l| l.name == UN).unwrap();
    assert!(un.stale_since.is_some());
    w.screen().await;

    let engagement = w.engagement().await;
    let res = w.activate(&w.partner, &engagement).await;
    res.assert_ok();
    assert_eq!(res.text(), format!("warn: {UN_DATED}"));
    assert_eq!(w.status(&engagement).await, EngagementStatus::Active);
    assert_eq!(w.history(&engagement).await, ["activate"]);

    w.page(&w.compliance).await.assert_ok().assert_see(NOTICE);
}

/// AC2: exactly fresh lists are not stale, so the outcome is `Allowed` and no
/// notice shows; a latest screening flagged stale still warns while the lists
/// are current.
#[renox::test]
async fn ac2_fresh_lists_are_allowed_and_a_flagged_screening_still_warns() {
    let w = world(StaleListPolicy::Warn).await;
    w.go_stale().await;
    // A little short of the 24 hour limit (ten minutes, so a slow test run
    // cannot cross it) is still fresh.
    let almost = w.now().await - renox::chrono::Duration::seconds(24 * 3600 - 600);
    for mut list in w.lists().await {
        list.last_updated_at = Some(almost);
        list.status = ListStatus::Current;
        list.stale_since = None;
        list.last_error = None;
        list.save(w.t.db()).await.unwrap();
    }
    let first = w.screen().await;
    assert!(!first.lists_stale);
    let engagement = w.engagement().await;
    let res = w.activate(&w.partner, &engagement).await;
    assert_eq!(res.text(), "active");
    assert_eq!(w.status(&engagement).await, EngagementStatus::Active);
    w.page(&w.compliance)
        .await
        .assert_ok()
        .assert_dont_see("screened on a stale list");

    // A party whose latest screening was flagged still gets a warning once
    // the lists are current again, with no re-run in between.
    let w = world(StaleListPolicy::Warn).await;
    w.go_stale().await;
    let flagged = w.screen().await;
    assert!(flagged.lists_stale);
    w.refresh(&[UN, UAE]).await;
    let engagement = w.engagement().await;
    let res = w.activate(&w.partner, &engagement).await;
    res.assert_ok();
    assert!(res.text().starts_with("warn: "), "{}", res.text());
    assert!(res.text().contains(UN_DATED), "{}", res.text());
    assert!(
        res.text()
            .contains("UAE Local Terrorist List of 2026-11-02"),
        "{}",
        res.text()
    );
    w.page(&w.compliance)
        .await
        .assert_see("screened on a stale list")
        .assert_see(UN_DATED);
}

/// AC3: with `Block` and the same stale UN list the activation is refused:
/// 422, naming the list and its date; the state is unchanged and no history
/// row is written.
#[renox::test]
async fn ac3_block_refuses_with_a_422_and_changes_nothing() {
    let w = world(StaleListPolicy::Block).await;
    w.go_stale().await;
    w.refresh(&[UAE]).await;
    w.screen().await;

    let engagement = w.engagement().await;
    let res = w.activate(&w.partner, &engagement).await;
    res.assert_status(422);
    assert!(res.text().contains(REFUSAL), "{}", res.text());
    assert_eq!(w.status(&engagement).await, EngagementStatus::Draft);
    assert!(w.history(&engagement).await.is_empty());
}

/// AC4: after a list update (the feed returns a new version) and the re-run,
/// which gives a new screening that is not flagged and is `clear`, the same
/// activation succeeds and the notice is gone.
#[renox::test]
async fn ac4_block_passes_after_the_update_and_the_rerun() {
    let w = world(StaleListPolicy::Block).await;
    w.go_stale().await;
    let flagged = w.screen().await;
    assert!(flagged.lists_stale);
    let engagement = w.engagement().await;
    w.activate(&w.partner, &engagement).await.assert_status(422);
    w.page(&w.compliance)
        .await
        .assert_see("screened on a stale list");

    // The check runs once an hour: the next one is an hour later.
    w.t.app.travel(Duration::from_secs(3600));
    w.feed.serve("v1");
    let report = w.freshness().await;
    assert_eq!(report.updated, 2);

    let all = w.screenings().await;
    let latest = all.last().unwrap();
    // One re-run per updated list; the last one runs on current lists.
    assert!(all.len() >= 2, "the re-run adds a screening");
    assert_ne!(latest.id, flagged.id);
    assert!(!latest.lists_stale);
    assert_eq!(latest.status, ScreeningStatus::Clear);

    let res = w.activate(&w.partner, &engagement).await;
    res.assert_ok();
    assert_eq!(res.text(), "active");
    assert_eq!(w.status(&engagement).await, EngagementStatus::Active);
    assert_eq!(w.history(&engagement).await, ["activate"]);
    w.page(&w.compliance)
        .await
        .assert_ok()
        .assert_dont_see("screened on a stale list");
}

/// AC5: a latest screening still flagged keeps the refusal although the lists
/// are current; with two lists, one stale, the refusal names only the stale
/// one, and each list once.
#[renox::test]
async fn ac5_block_keeps_refusing_a_flagged_screening_and_names_each_stale_list_once() {
    let w = world(StaleListPolicy::Block).await;
    w.go_stale().await;
    w.screen().await;
    // Both lists are current now, the re-run is not done.
    w.refresh(&[UN, UAE]).await;
    let engagement = w.engagement().await;
    let res = w.activate(&w.partner, &engagement).await;
    res.assert_status(422);
    assert!(res.text().contains(UN_DATED), "{}", res.text());
    assert_eq!(w.status(&engagement).await, EngagementStatus::Draft);

    // One stale list of two, flagged in the latest screening as well: the UN
    // list is named once and the current one not at all.
    let w = world(StaleListPolicy::Block).await;
    w.go_stale().await;
    w.refresh(&[UAE]).await;
    w.screen().await;
    let engagement = w.engagement().await;
    let res = w.activate(&w.partner, &engagement).await;
    res.assert_status(422);
    let text = res.text();
    assert!(text.contains(REFUSAL), "{text}");
    assert_eq!(text.matches(UN).count(), 1, "{text}");
    assert!(!text.contains(UAE), "{text}");
}

/// AC6: warn mode never refuses, with any number of stale lists; the party
/// stays unchanged (no block, no event, no screening written by the guard).
#[renox::test]
async fn ac6_warn_never_refuses_and_leaves_the_party_unchanged() {
    let w = world(StaleListPolicy::Warn).await;
    w.go_stale().await;
    let before = w.t.reload(&w.party).await;
    let screenings = w.screenings().await.len();

    // Two stale lists, then again after one is current: never refused.
    for refreshed in [&[][..], &[UAE][..]] {
        w.refresh(refreshed).await;
        let engagement = w.engagement().await;
        let res = w.activate(&w.partner, &engagement).await;
        res.assert_ok();
        assert!(res.text().starts_with("warn: "), "{}", res.text());
        assert!(res.text().contains(UN), "{}", res.text());
        assert_eq!(w.status(&engagement).await, EngagementStatus::Active);
    }

    let after = w.t.reload(&w.party).await;
    assert_eq!(after.status, before.status);
    assert_eq!(after.updated_at, before.updated_at);
    assert!(w.t.blocked().is_empty() && w.t.unblocked().is_empty());
    assert_eq!(
        w.screenings().await.len(),
        screenings,
        "the guard screens nothing"
    );
}

/// AC7 (module side): with screening off, the guard is the no-op even in
/// block mode, and the transition runs directly. The `parties` module off is
/// in `crates/wl-kernel/tests/screening_guard.rs`.
#[renox::test]
async fn ac7_with_screening_off_the_transition_runs_directly() {
    let module = WlParties::new().stale_list_at_onboarding(StaleListPolicy::Block);
    let w = world_with(module, FakeFeed::default()).await;
    let engagement = w.engagement().await;
    let res = w.activate(&w.partner, &engagement).await;
    res.assert_ok();
    assert_eq!(res.text(), "active");
    assert_eq!(w.status(&engagement).await, EngagementStatus::Active);
    assert_eq!(w.history(&engagement).await, ["activate"]);
    w.page(&w.compliance)
        .await
        .assert_ok()
        .assert_dont_see("screened on a stale list");
}

/// AC8: without a screening permission the party page shows no stale notice,
/// and the refusal (shown to the partner who runs the transition) carries
/// nothing but the lists: no candidate, match or block reason.
#[renox::test]
async fn ac8_the_notice_is_hidden_and_the_refusal_names_no_match() {
    let w = world(StaleListPolicy::Block).await;
    w.go_stale().await;
    w.screen().await;

    w.page(&w.compliance)
        .await
        .assert_ok()
        .assert_see("screened on a stale list")
        .assert_see(UN_DATED);
    w.page(&w.partner)
        .await
        .assert_ok()
        .assert_dont_see("screened on a stale list")
        .assert_dont_see(UN);

    let engagement = w.engagement().await;
    let res = w.activate(&w.partner, &engagement).await;
    res.assert_status(422);
    // The error page's own text, not the page around it.
    let page = res.text();
    let text = page
        .split("<h2>Error</h2><pre>")
        .nth(1)
        .and_then(|rest| rest.split("</pre>").next())
        .unwrap_or_else(|| panic!("no error text in {page}"))
        .to_owned();
    assert!(
        text.starts_with("Screening list is out of date: ") && text.contains(UN_DATED),
        "{text}"
    );
    let lower = text.to_lowercase();
    for word in ["candidate", "match", "block", "reason", "sanction"] {
        assert!(!lower.contains(word), "`{word}` in `{text}`");
    }
}
